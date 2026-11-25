mod loop_run_algorithm;

use crate::api;
use crate::config;
use crate::database;
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
    deposit_7d: f64,
    amount_buy_min: Option<f64>,
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

        let deposit_7d: f64 = config
            .get_config("deposit_7d")?
            .ok_or_else(|| anyhow::anyhow!("Missing 'deposit_7d' in config"))?;
        let amount_buy_min: Option<f64> = config.get_config("amount_buy_min")?;

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
            deposit_7d,
            amount_buy_min,
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

    pub async fn loop_log_prices(&self) {
        loop {
            match self.client_cex.get_price_coin("btc").await {
                Ok(Some(price_info)) => {
                    let parse_price = |key: &str| -> Option<f64> {
                        price_info[key].as_str()?.parse::<f64>().ok()
                    };

                    let price_last = parse_price("last");
                    let price_buy = parse_price("ask");
                    let price_sell = parse_price("bid");

                    match (price_last, price_buy, price_sell) {
                        (Some(price_last), Some(price_buy), Some(price_sell)) => {
                            if let Err(e) = self.database
                                .save_price("btc", price_buy, price_sell, price_last)
                            {
                                log::error!("Failed to log BTC prices: {}", e);
                            } else {
                                log::info!(
                                    "Logged BTC prices - Buy: {}, Sell: {}, Last: {}",
                                    price_buy,
                                    price_sell,
                                    price_last
                                );
                            }
                        }
                        _ => log::warn!("One or more BTC price fields are missing or invalid."),
                    }
                }
                Ok(None) => log::warn!("Price info not found for BTC."),
                Err(e) => log::error!("Error fetching BTC price: {}", e),
            }

            let time_sleep_seconds = self.get_seconds_to_timestep_next().await;
            log::info!(
                "Sleeping for {} seconds until next timestep.",
                time_sleep_seconds
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(time_sleep_seconds.into())).await;
        }
    }
}
