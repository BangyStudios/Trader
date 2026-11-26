// Run from the project root:
// cargo run --bin simulator -- <n_days_last> [algorithm_name] [steps_csv_path] [deposit_initial] [deposit_1d_simulated]
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
use crate::structure::common::Amount;
use crate::structure::timeseries::{PriceRow, Timeseries, TimeseriesRow};

use anyhow::anyhow;
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};

const days_history_closing: i32 = 365 * 8 + 3; // Matches the daemon's closing-price history
const days_history_intraday: i32 = 30; // Matches the daemon's intraday window
const algorithm_name_default: &str = "v1_0";
const currency_market: &str = "aud"; // Currency of the balance, deposits and prices
const coin: &str = "btc";

/// One row per simulated timestep for plotting; `pct_buy` is the v1_0 weighted percentile, `None` when
/// the percentile cannot be computed
struct SimulatorStep {
    timestamp: NaiveDateTime,
    pct_buy: Option<f64>,
    amount: Amount, // Bought at this timestep, in the market currency
    price_buy: f64,
    price_sell: f64,
    price_ma: f64, // 4-year moving average of daily closes
    deposit: Amount, // Deposit added at this timestep, 0 on all but the first of each day (and the initial deposit)
}

struct SimulatorResult {
    steps: Vec<SimulatorStep>,
    n_days_last: i64,
    amount_deposited: Amount,
    amount_spent: Amount,
    balance: Amount,
    amount_held: Amount, // Coin bought by the algorithm
    amount_held_dca: Amount, // Baseline that spends each deposit at the first timestep of its day
    price_sell_final: f64,
    n_purchases: usize,
}

fn init_algorithm(algorithm_name: &str) -> anyhow::Result<Box<dyn Algorithm>> {
    match algorithm_name {
        "v1_0" => Ok(Box::new(Algorithm_v1_0::init())),
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
    purchases: &Timeseries<(Amount, Amount, f64)>,
    datetime_now: NaiveDateTime,
) -> Timeseries<(Amount, Amount, f64)> {
    let datetime_start = datetime_now - Duration::days(7);
    let mut purchases_7d: Timeseries<(Amount, Amount, f64)> = Timeseries::new(purchases.columns.clone());
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

/// Replays every intraday price row of the last `n_days_last` days as one timestep, adding `deposit_initial`
/// to the balance on the first timestep and `deposit_1d_simulated` on the first timestep of each day. The
/// algorithm still receives `deposit_1d` as its spend rate reference. Buys fill at `price_buy`; sells are not
/// simulated, matching the daemon
async fn run_simulation(
    algorithm: &dyn Algorithm,
    prices_closing: &[PriceRow],
    prices_intraday: &[PriceRow],
    n_days_last: i64,
    deposit_1d: &Amount,
    deposit_initial: &Amount,
    deposit_1d_simulated: &Amount,
    amount_buy_min: &Option<Amount>,
) -> anyhow::Result<SimulatorResult> {
    let price_row_last = match prices_intraday.last() {
        Some(price_row_last) => price_row_last,
        None => return Err(anyhow!("No intraday prices in the database")),
    };
    let datetime_start = price_row_last.timestamp - Duration::days(n_days_last);

    let mut balance = Amount::init(currency_market, 0.0);
    let mut amount_held = Amount::init(coin, 0.0);
    let mut amount_held_dca = Amount::init(coin, 0.0);
    let mut amount_deposited = Amount::init(currency_market, 0.0);
    let mut amount_spent = Amount::init(currency_market, 0.0);
    let mut purchases: Timeseries<(Amount, Amount, f64)> = Timeseries::new(vec![
        "amount_out".to_string(),
        "amount_in".to_string(),
        "price".to_string(),
    ]);
    let mut steps: Vec<SimulatorStep> = Vec::new();
    let mut date_deposit_last: Option<NaiveDate> = None;
    let mut is_deposited_initial = false;
    let mut date_ma_4y: Option<NaiveDate> = None;
    let mut ma_4y: Option<Timeseries<(f64, f64, f64)>> = None;

    for index_now in 0..prices_intraday.len() {
        let price_row = &prices_intraday[index_now];
        let datetime_now = price_row.timestamp;
        if datetime_now < datetime_start {
            continue;
        }
        let date_now = datetime_now.date();

        let mut deposit = Amount::init(currency_market, 0.0);
        if !is_deposited_initial {
            deposit.amount += deposit_initial.amount;
            is_deposited_initial = true;
        }
        if date_deposit_last != Some(date_now) {
            deposit.amount += deposit_1d_simulated.amount;
            date_deposit_last = Some(date_now);
        }
        if deposit.amount > 0.0 {
            balance.amount += deposit.amount;
            amount_deposited.amount += deposit.amount;
            amount_held_dca.amount += deposit.amount / price_row.price_buy;
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
            amount: Amount::init(currency_market, 0.0),
            price_buy: price_row.price_buy,
            price_sell: price_row.price_sell,
            price_ma,
            deposit,
        });

        let params = AlgorithmParams {
            deposit_1d: deposit_1d.clone(),
            amount_buy_min: amount_buy_min.clone(),
        };
        let input = AlgorithmInput {
            datetime_now,
            balance: balance.clone(),
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
        let amount = Amount::init(&amount.currency, (amount.amount * 100.0).floor() / 100.0);
        if amount.amount <= 0.0 {
            continue;
        }
        if amount.currency != balance.currency {
            log::warn!("Buy of {} is not in the balance currency at {}", amount, datetime_now);
            continue;
        }
        if amount.amount > balance.amount {
            log::warn!("Buy of {:.2} exceeds balance {:.2} at {}", amount, balance, datetime_now);
            continue;
        }

        let amount_in = Amount::init(coin, amount.amount / price_row.price_buy);
        balance.amount -= amount.amount;
        amount_held.amount += amount_in.amount;
        amount_spent.amount += amount.amount;
        let step_last = steps.last_mut().expect("A step is pushed before every buy");
        step_last.amount = amount.clone();
        purchases.data.push(TimeseriesRow {
            timestamp: datetime_now,
            value: (amount, amount_in, price_row.price_buy),
        });
    }

    Ok(SimulatorResult {
        steps,
        n_days_last,
        amount_deposited,
        amount_spent,
        balance,
        amount_held,
        amount_held_dca,
        price_sell_final: price_row_last.price_sell,
        n_purchases: purchases.len(),
    })
}

/// Writes timestamp, pct_buy (blank when `None`) and the amount bought per timestep. The `amount_aud` and
/// `deposit_aud` headers are kept for existing plots
fn save_simulator_steps(steps: &[SimulatorStep], path_csv: &str) -> anyhow::Result<()> {
    let mut lines: Vec<String> = vec!["timestamp,pct_buy,amount_aud,price_buy,price_sell,price_ma,deposit_aud".to_string()];
    for step in steps {
        let pct_buy_str = match step.pct_buy {
            Some(pct_buy) => pct_buy.to_string(),
            None => String::new(),
        };
        lines.push(format!(
            "{},{},{},{},{},{},{}",
            step.timestamp, pct_buy_str, step.amount.amount, step.price_buy, step.price_sell, step.price_ma, step.deposit.amount
        ));
    }
    std::fs::write(path_csv, lines.join("\n"))?;
    Ok(())
}

fn log_simulator_result(result: &SimulatorResult) -> () {
    let currency = &result.balance.currency;
    let value_held = Amount::init(currency, result.amount_held.amount * result.price_sell_final);
    let value_total = Amount::init(currency, result.balance.amount + value_held.amount);
    let profit = Amount::init(currency, value_total.amount - result.amount_deposited.amount);

    let mut profit_pct = 0.0;
    if result.amount_deposited.amount > 0.0 {
        profit_pct = 100.0 * profit.amount / result.amount_deposited.amount;
    }
    let mut price_average = Amount::init(currency, 0.0);
    if result.amount_held.amount > 0.0 {
        price_average.amount = result.amount_spent.amount / result.amount_held.amount;
    }
    let price_sell_final = Amount::init(currency, result.price_sell_final);

    log::info!("Simulation over the last {} day(s)", result.n_days_last);
    log::info!("Deposited {:.2}, spent {:.2} in {} purchase(s)", result.amount_deposited, result.amount_spent, result.n_purchases);
    log::info!("Holding {:.8} at an average {:.2}, final sell price {:.2}", result.amount_held, price_average, price_sell_final);
    let value_dca = Amount::init(currency, result.amount_held_dca.amount * result.price_sell_final);
    let mut price_average_dca = Amount::init(currency, 0.0);
    if result.amount_held_dca.amount > 0.0 {
        price_average_dca.amount = result.amount_deposited.amount / result.amount_held_dca.amount;
    }
    let value_vs_dca = Amount::init(currency, value_total.amount - value_dca.amount);
    log::info!("DCA baseline: {:.8} at an average {:.2}, final value {:.2}, algorithm minus DCA {:.2}", result.amount_held_dca, price_average_dca, value_dca, value_vs_dca);
    log::info!("Final value {:.2} (balance {:.2} + held {:.2}), profit {:.2} ({:.2}%)", value_total, result.balance, value_held, profit, profit_pct);
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
    let amount_buy_min = match amount_buy_min {
        Some(amount_buy_min) => Some(Amount::init(currency_market, amount_buy_min)),
        None => None,
    };

    let deposit_initial: f64 = match args.get(4) {
        Some(deposit_initial_str) => deposit_initial_str
            .parse()
            .map_err(|e| anyhow!("Invalid deposit_initial '{}': {}", deposit_initial_str, e))?,
        None => 0.0,
    };
    let deposit_1d_simulated: f64 = match args.get(5) {
        Some(deposit_1d_simulated_str) => deposit_1d_simulated_str
            .parse()
            .map_err(|e| anyhow!("Invalid deposit_1d_simulated '{}': {}", deposit_1d_simulated_str, e))?,
        None => deposit_1d,
    };
    let deposit_1d = Amount::init(currency_market, deposit_1d);
    let deposit_initial = Amount::init(currency_market, deposit_initial);
    let deposit_1d_simulated = Amount::init(currency_market, deposit_1d_simulated);

    let database = Database::init(config.clone())?;
    let days_closing = n_days_last as i32 + days_history_closing;
    let days_intraday = n_days_last as i32 + days_history_intraday;
    let prices_closing = database.get_prices_last(coin, days_closing, true)?;
    let prices_intraday = database.get_prices_last(coin, days_intraday, false)?;
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
        &deposit_1d,
        &deposit_initial,
        &deposit_1d_simulated,
        &amount_buy_min,
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
