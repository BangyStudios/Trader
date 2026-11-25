// Run from the project root: cargo run --bin simulator -- <n_days_last> [algorithm_name] [steps_csv_path]
mod algorithm;
mod api;
mod config;
mod database;
mod quant;
mod structure;
mod utils;

use crate::algorithm::v1_0::{get_pct_buy_datetime, Algorithm_v1_0};
use crate::algorithm::Algorithm;
use crate::config::Config;
use crate::database::Database;
use crate::quant::moving_average::get_timeseries_ma_trailing;
use crate::structure::algorithm::{AlgorithmInput, AlgorithmParams};
use crate::structure::timeseries::{PriceRow, Timeseries, TimeseriesRow};

use anyhow::anyhow;
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};

const days_history_closing: i32 = 365 * 8 + 3; // Matches the daemon's closing-price history
const days_history_intraday: i32 = 30; // Matches the daemon's intraday window
const algorithm_name_default: &str = "v1_0";

/// One row per simulated timestep for plotting; `pct_buy` is the v1_0 weighted percentile, `None` when
/// the percentile cannot be computed
struct SimulatorStep {
    timestamp: NaiveDateTime,
    pct_buy: Option<f64>,
    amount_aud: f64,
    price_buy: f64,
    price_sell: f64,
    price_ma: f64, // 4-year moving average of daily closes
    deposit_aud: f64, // Deposit added at this timestep, 0 on all but the first of each day
}

struct SimulatorResult {
    steps: Vec<SimulatorStep>,
    n_days_last: i64,
    amount_deposited: f64,
    amount_spent: f64,
    balance: f64,
    amount_btc: f64,
    amount_btc_dca: f64, // Baseline that spends each deposit at the first timestep of its day
    price_sell_final: f64,
    n_purchases: usize,
}

fn init_algorithm(algorithm_name: &str) -> anyhow::Result<Box<dyn Algorithm>> {
    match algorithm_name {
        "v1_0" => Ok(Box::new(Algorithm_v1_0 {})),
        _ => Err(anyhow!("Unsupported algorithm '{}'", algorithm_name)),
    }
}

/// Daily moving averages built only from closes before midnight of `datetime_now`, so no future prices leak in
fn get_ma_4y_datetime(
    prices_closing: &[PriceRow],
    datetime_now: NaiveDateTime,
) -> anyhow::Result<Timeseries<(f64, f64, f64)>> {
    let datetime_midnight = datetime_now.date().and_time(NaiveTime::MIN);
    let mut prices_known: Vec<PriceRow> = Vec::new();
    for price_row in prices_closing {
        if price_row.timestamp >= datetime_midnight {
            continue;
        }
        prices_known.push(price_row.clone());
    }

    let len_window = 365 * 4 + 1;
    let ma_4y = get_timeseries_ma_trailing(prices_known, "price_mid", len_window)?;
    Ok(ma_4y)
}

/// Intraday rows in the last 30 days up to and including `prices_intraday[index_now]`
fn get_prices_30d_index(prices_intraday: &[PriceRow], index_now: usize) -> Vec<PriceRow> {
    let datetime_now = prices_intraday[index_now].timestamp;
    let datetime_start = datetime_now - Duration::days(days_history_intraday as i64);

    let mut prices_30d: Vec<PriceRow> = Vec::new();
    for index in (0..=index_now).rev() {
        let price_row = &prices_intraday[index];
        if price_row.timestamp < datetime_start {
            break;
        }
        prices_30d.push(price_row.clone());
    }
    prices_30d.reverse();
    prices_30d
}

fn get_purchases_7d_datetime(
    purchases: &Timeseries<(f64, f64, f64)>,
    datetime_now: NaiveDateTime,
) -> Timeseries<(f64, f64, f64)> {
    let datetime_start = datetime_now - Duration::days(7);
    let mut purchases_7d: Timeseries<(f64, f64, f64)> = Timeseries::new(purchases.columns.clone());
    for purchase_row in purchases.range(datetime_start, datetime_now) {
        purchases_7d.data.push(purchase_row.clone());
    }
    purchases_7d
}

fn get_pct_buy_step(
    ma_4y: &Timeseries<(f64, f64, f64)>,
    prices_30d: &[PriceRow],
    price_row: &PriceRow,
) -> Option<f64> {
    let ma_4y_latest = ma_4y.latest()?;
    let price_mid = (price_row.price_buy + price_row.price_sell) / 2.0;
    let prop_mid = price_mid / ma_4y_latest.value.0;

    match get_pct_buy_datetime(ma_4y, prices_30d, price_mid, prop_mid, price_row.timestamp) {
        Ok(pct_buy) => Some(pct_buy),
        Err(_) => None,
    }
}

/// Replays every intraday price row of the last `n_days_last` days as one timestep, adding `deposit_1d`
/// to the balance on the first timestep of each day. Buys fill at `price_buy`; sells are not simulated,
/// matching the daemon
async fn run_simulation(
    algorithm: &dyn Algorithm,
    prices_closing: &[PriceRow],
    prices_intraday: &[PriceRow],
    n_days_last: i64,
    deposit_1d: f64,
    amount_buy_min: Option<f64>,
) -> anyhow::Result<SimulatorResult> {
    let price_row_last = match prices_intraday.last() {
        Some(price_row_last) => price_row_last,
        None => return Err(anyhow!("No intraday prices in the database")),
    };
    let datetime_start = price_row_last.timestamp - Duration::days(n_days_last);

    let mut balance = 0.0;
    let mut amount_btc = 0.0;
    let mut amount_btc_dca = 0.0;
    let mut amount_deposited = 0.0;
    let mut amount_spent = 0.0;
    let mut purchases: Timeseries<(f64, f64, f64)> = Timeseries::new(vec![
        "amount_aud".to_string(),
        "amount_btc".to_string(),
        "price".to_string(),
    ]);
    let mut steps: Vec<SimulatorStep> = Vec::new();
    let mut date_deposit_last: Option<NaiveDate> = None;
    let mut date_ma_4y: Option<NaiveDate> = None;
    let mut ma_4y: Option<Timeseries<(f64, f64, f64)>> = None;

    for index_now in 0..prices_intraday.len() {
        let price_row = &prices_intraday[index_now];
        let datetime_now = price_row.timestamp;
        if datetime_now < datetime_start {
            continue;
        }
        let date_now = datetime_now.date();

        let mut deposit_aud = 0.0;
        if date_deposit_last != Some(date_now) {
            deposit_aud = deposit_1d;
            balance += deposit_1d;
            amount_deposited += deposit_1d;
            amount_btc_dca += deposit_1d / price_row.price_buy;
            date_deposit_last = Some(date_now);
        }

        // Closes only change at midnight, so the moving averages are rebuilt once per day
        if date_ma_4y != Some(date_now) {
            date_ma_4y = Some(date_now);
            ma_4y = match get_ma_4y_datetime(prices_closing, datetime_now) {
                Ok(ma_4y_now) => Some(ma_4y_now),
                Err(e) => {
                    log::warn!("Skipping {}: {}", date_now, e);
                    None
                }
            };
        }
        let ma_4y_now = match &ma_4y {
            Some(ma_4y_now) => ma_4y_now.clone(),
            None => continue,
        };

        let prices_30d = get_prices_30d_index(prices_intraday, index_now);
        let pct_buy = get_pct_buy_step(&ma_4y_now, &prices_30d, price_row);
        let ma_4y_latest = ma_4y_now.latest().expect("get_ma_4y_datetime returns at least one row");
        let price_ma = ma_4y_latest.value.0;
        steps.push(SimulatorStep {
            timestamp: datetime_now,
            pct_buy,
            amount_aud: 0.0,
            price_buy: price_row.price_buy,
            price_sell: price_row.price_sell,
            price_ma,
            deposit_aud,
        });

        let params = AlgorithmParams { deposit_1d, amount_buy_min };
        let input = AlgorithmInput {
            datetime_now,
            balance,
            price_buy: price_row.price_buy,
            price_sell: price_row.price_sell,
            ma_4y: ma_4y_now,
            prices_30d,
            purchases_7d: get_purchases_7d_datetime(&purchases, datetime_now),
        };
        let command = match algorithm.run_algorithm(params, input).await {
            Ok(command) => command,
            Err(e) => {
                log::warn!("Algorithm error at {}: {}", datetime_now, e);
                continue;
            }
        };
        let amount = match (command.command, command.amount) {
            (1, Some(amount)) => amount,
            _ => continue,
        };

        // Same 2 decimal floor as the daemon, so the order never overspends
        let amount_aud = (amount * 100.0).floor() / 100.0;
        if amount_aud <= 0.0 {
            continue;
        }
        if amount_aud > balance {
            log::warn!("Buy of {:.2} AUD exceeds balance {:.2} AUD at {}", amount_aud, balance, datetime_now);
            continue;
        }

        let amount_btc_bought = amount_aud / price_row.price_buy;
        balance -= amount_aud;
        amount_btc += amount_btc_bought;
        amount_spent += amount_aud;
        let step_last = steps.last_mut().expect("A step is pushed before every buy");
        step_last.amount_aud = amount_aud;
        purchases.data.push(TimeseriesRow {
            timestamp: datetime_now,
            value: (amount_aud, amount_btc_bought, price_row.price_buy),
        });
    }

    Ok(SimulatorResult {
        steps,
        n_days_last,
        amount_deposited,
        amount_spent,
        balance,
        amount_btc,
        amount_btc_dca,
        price_sell_final: price_row_last.price_sell,
        n_purchases: purchases.len(),
    })
}

/// Writes timestamp, pct_buy (blank when `None`) and amount_aud per timestep
fn save_simulator_steps(steps: &[SimulatorStep], path_csv: &str) -> anyhow::Result<()> {
    let mut lines: Vec<String> = vec!["timestamp,pct_buy,amount_aud,price_buy,price_sell,price_ma,deposit_aud".to_string()];
    for step in steps {
        let pct_buy_str = match step.pct_buy {
            Some(pct_buy) => pct_buy.to_string(),
            None => String::new(),
        };
        lines.push(format!(
            "{},{},{},{},{},{},{}",
            step.timestamp, pct_buy_str, step.amount_aud, step.price_buy, step.price_sell, step.price_ma, step.deposit_aud
        ));
    }
    std::fs::write(path_csv, lines.join("\n"))?;
    Ok(())
}

fn log_simulator_result(result: &SimulatorResult) -> () {
    let value_btc = result.amount_btc * result.price_sell_final;
    let value_total = result.balance + value_btc;
    let profit = value_total - result.amount_deposited;

    let mut profit_pct = 0.0;
    if result.amount_deposited > 0.0 {
        profit_pct = 100.0 * profit / result.amount_deposited;
    }
    let mut price_average = 0.0;
    if result.amount_btc > 0.0 {
        price_average = result.amount_spent / result.amount_btc;
    }

    log::info!("Simulation over the last {} day(s)", result.n_days_last);
    log::info!("Deposited {:.2} AUD, spent {:.2} AUD in {} purchase(s)", result.amount_deposited, result.amount_spent, result.n_purchases);
    log::info!("Holding {:.8} BTC at an average {:.2} AUD, final sell price {:.2} AUD", result.amount_btc, price_average, result.price_sell_final);
    let value_dca = result.amount_btc_dca * result.price_sell_final;
    let mut price_average_dca = 0.0;
    if result.amount_btc_dca > 0.0 {
        price_average_dca = result.amount_deposited / result.amount_btc_dca;
    }
    log::info!("DCA baseline: {:.8} BTC at an average {:.2} AUD, final value {:.2} AUD, algorithm minus DCA {:.2} AUD", result.amount_btc_dca, price_average_dca, value_dca, value_total - value_dca);
    log::info!("Final value {:.2} AUD (balance {:.2} + BTC {:.2}), profit {:.2} AUD ({:.2}%)", value_total, result.balance, value_btc, profit, profit_pct);
}

async fn run_simulator() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let config = Config::init();

    let n_days_last_str: Option<String> = args.get(1).cloned();
    let n_days_last: i64 = match n_days_last_str {
        Some(n_days_last_str) => n_days_last_str
            .parse()
            .map_err(|e| anyhow!("Invalid n_days_last '{}': {}", n_days_last_str, e))?,
        None => config
            .get_config("n_days_last")?
            .ok_or_else(|| anyhow!("Missing n_days_last argument or 'n_days_last' in config"))?,
    };
    if n_days_last <= 0 {
        return Err(anyhow!("Invalid n_days_last '{}': must be positive", n_days_last));
    }

    let algorithm_name: String = match args.get(2) {
        Some(algorithm_name) => algorithm_name.clone(),
        None => algorithm_name_default.to_string(),
    };
    let algorithm = init_algorithm(&algorithm_name)?;

    let deposit_1d: f64 = config
        .get_config("deposit_1d")?
        .ok_or_else(|| anyhow!("Missing 'deposit_1d' in config"))?;
    let amount_buy_min: Option<f64> = config.get_config("amount_buy_min")?;

    let database = Database::init(config.clone())?;
    let days_closing = n_days_last as i32 + days_history_closing;
    let days_intraday = n_days_last as i32 + days_history_intraday;
    let prices_closing = database.get_prices_last("btc", days_closing, true)?;
    let prices_intraday = database.get_prices_last("btc", days_intraday, false)?;
    log::info!(
        "Loaded {} closing and {} intraday price rows, running '{}'",
        prices_closing.len(),
        prices_intraday.len(),
        algorithm_name
    );

    let result = run_simulation(
        algorithm.as_ref(),
        &prices_closing,
        &prices_intraday,
        n_days_last,
        deposit_1d,
        amount_buy_min,
    )
    .await?;
    log_simulator_result(&result);
    if let Some(path_csv) = args.get(3) {
        save_simulator_steps(&result.steps, path_csv)?;
        log::info!("Saved {} step(s) to {}", result.steps.len(), path_csv);
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    // The algorithms log every timestep at info, so only the simulator itself defaults to info
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,simulator=info,simulator::algorithm=warn")).init();

    match run_simulator().await {
        Ok(()) => {}
        Err(e) => {
            log::error!("Simulator failed: {}", e);
            std::process::exit(1);
        }
    }
}
