mod loop_log_prices;
mod loop_run_algorithm;

use crate::api;
use crate::config;
use crate::database;
use crate::structure::common::Amount;
use crate::quant::moving_average::get_timeseries_diff;
use crate::utils;

use chrono::{DateTime, Timelike, Utc};
use std::sync::Arc;
use tokio::join;

pub struct Daemon {
    config: config::Config,
    database: database::Database,
    client_cex: Box<dyn api::CExClient>,
    timestep_mins: u32,
    currency_market: String, // Currency that the balance, deposits and orders are in
    deposit_1d: Amount,
    amount_buy_min: Option<Amount>,
    balance_reserve: Amount,
    is_wet_run: bool, // Places real orders when true, otherwise only logs them
}

impl Daemon {
    pub fn new(client_cex_name: &str) -> anyhow::Result<Self> {
        let config = config::Config::init();
        let database = database::Database::init(config.clone())?;

        let client_cex = match client_cex_name {
            "coinspot" => api::coinspot::init_cex_client(&config)?,
            _ => {
                return Err(anyhow::format_err!(
                    "Unsupported CEx client: {}",
                    client_cex_name
                ))
            }
        };

        let timestep_mins: u32 = config.get_config("timestep_mins")?.unwrap_or(5);

        let currency_market = "aud".to_string();
        let deposit_1d: f64 = config
            .get_config("deposit_1d")?
            .ok_or_else(|| anyhow::anyhow!("Missing 'deposit_1d' in config"))?;
        let deposit_1d = Amount::init(&currency_market, deposit_1d);
        let amount_buy_min: Option<f64> = config.get_config("amount_buy_min")?;
        let amount_buy_min = match amount_buy_min {
            Some(amount_buy_min) => Some(Amount::init(&currency_market, amount_buy_min)),
            None => None,
        };
        // Missing means no reserve
        let balance_reserve: f64 = config.get_config("balance_reserve")?.unwrap_or(0.0);
        let balance_reserve = Amount::init(&currency_market, balance_reserve);
        log::info!("Balance reserve: {:.2}", balance_reserve);

        // Missing means dry run, so an incomplete config never trades
        let is_wet_run: bool = config.get_config("is_wet_run")?.unwrap_or(false);
        log::info!(
            "Algorithm mode: {}",
            if is_wet_run { "WET RUN (real orders)" } else { "dry run" }
        );

        return Ok(Daemon {
            config,
            database,
            client_cex,
            timestep_mins,
            currency_market,
            deposit_1d,
            amount_buy_min,
            balance_reserve,
            is_wet_run,
        });
    }

    pub async fn run(&self) {
        let (
            result_log_prices, 
            result_run_algorithm
        ) = join!(
            self.loop_log_prices(), 
            self.loop_run_algorithm()
        );
    }

    pub async fn get_seconds_to_timestep_next(&self) -> u32 {
        let datetime_now = Utc::now();

        let minute_now = datetime_now.minute();
        let second_now = datetime_now.second();
        let minutes_past = minute_now % self.timestep_mins;

        let mut sync = 0;

        let minutes_to_add = if minutes_past == self.timestep_mins - 1 && second_now >= 55 {
            // Within last 5 seconds of the interval, wait full interval
            sync = second_now as i32 - 60;
            self.timestep_mins
        } else if minutes_past == 0 && second_now == 0 {
            // Exactly at boundary, go to next interval
            self.timestep_mins
        } else {
            // Not at boundary, go to next boundary
            self.timestep_mins - minutes_past
        };

        let next_timestep = if sync == 0 {
            (datetime_now + chrono::Duration::minutes(minutes_to_add as i64))
                .with_second(0)
                .and_then(|dt| dt.with_nanosecond(0))
                .unwrap()
        } else {
            (datetime_now
                + chrono::Duration::minutes(minutes_to_add as i64)
                + chrono::Duration::minutes(if sync < 0 { 1 } else { 0 } as i64))
            .with_second(0)
            .and_then(|dt| dt.with_nanosecond(0))
            .unwrap()
        };

        let duration = next_timestep.signed_duration_since(datetime_now);
        duration.num_seconds().max(0) as u32 // Always at least 1 second
    }
}
