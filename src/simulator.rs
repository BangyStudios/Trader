// Run from the project root:
// cargo run --bin simulator -- <n_days_last> [algorithm_name] [steps_csv_path] [deposit_initial] [deposit_1d_simulated] [balance_reserve]
// cargo run --release --bin simulator -- sweep <grid_toml_path> <results_csv_path>
mod algorithm;
mod api;
mod config;
mod database;
mod quant;
mod structure;
mod utils;

use crate::algorithm::v1_0::{get_pct_buy_datetime, Algorithm_v1_0, Hyperparameters_v1_0};
use crate::algorithm::Algorithm;
use crate::config::Config;
use crate::database::Database;
use crate::quant::moving_average::get_timeseries_ma_trailing;
use crate::structure::algorithm::{AlgorithmInput, AlgorithmParams};
use crate::structure::common::Amount;
use crate::structure::timeseries::{PriceRow, Timeseries, TimeseriesRow};

use anyhow::anyhow;
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::Deserialize;
use std::collections::HashMap;

const days_history_closing: i32 = 365 * 8 + 3; // Matches the daemon's closing-price history
const days_history_intraday: i32 = 30; // Matches the daemon's intraday window
const algorithm_name_default: &str = "v1_0";
const currency_market: &str = "aud"; // Currency of the balance, deposits and prices
const coin: &str = "btc";

/// One row per simulated timestep for plotting; `pct_buy` is the v1_0 weighted percentile, `None` when
/// the percentile cannot be computed or is not saved
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
    datetime_start: NaiveDateTime,
    datetime_end: NaiveDateTime,
    amount_deposited: Amount,
    amount_spent: Amount,
    amount_spent_first_half: Amount, // Spent from 00:00 to 12:00 UTC, the first half of each deposit's day
    balance: Amount,
    balance_reserve: Amount,
    amount_held: Amount, // Coin bought by the algorithm
    amount_held_dca: Amount, // Baseline that spends each deposit at the first timestep of its day
    price_sell_final: f64,
    price_buy_mean: f64, // Mean over the timesteps, the price an evenly spread buyer pays
    n_purchases: usize,
    n_steps_below_reserve: usize, // Timesteps ending with the balance below `balance_reserve`
}

/// Returns on the amount deposited, with the final balance valued as cash
struct SimulatorMetrics {
    roi: f64,
    roi_dca: f64,
    // As `roi`, but valuing the final balance as if bought at `price_buy_mean`, so cash held at the end neither
    // gains nor loses from the final price
    roi_neutral: f64,
    prop_steps_below_reserve: f64,
    prop_spent_first_half: f64,
}

/// Price history shared by every run: intraday rows and the 4-year moving averages known on each date
struct SimulatorPrices {
    prices_intraday: Vec<PriceRow>,
    ma_4y_by_date: HashMap<NaiveDate, Timeseries<(f64, f64, f64)>>, // Missing on dates without enough closes
}

/// Ranges of v1_0 hyperparameters and reserves to backtest over sliding windows. Every combination runs on every window
#[derive(Deserialize)]
struct SweepGrid {
    deposit_1d: f64,
    amount_buy_min: Option<f64>,
    n_days_last: i64, // Windows tile the last `n_days_last` days
    n_days_window: i64,
    n_days_window_step: i64,
    balance_reserve: Vec<f64>,
    multiplier_max: Vec<f64>,
    pct_neutral: Vec<f64>,
    prop_balance_1d_max: Vec<f64>,
    prop_budget_step_max: Vec<f64>,
    pct_reserve_unlock: Vec<f64>,
    days_runway: Vec<f64>,
}

#[derive(Clone)]
struct SweepCase {
    balance_reserve: f64,
    hyperparameters: Hyperparameters_v1_0,
}

struct SweepJob {
    index_case: usize,
    datetime_start: NaiveDateTime,
    datetime_end: NaiveDateTime,
}

fn init_algorithm(algorithm_name: &str) -> anyhow::Result<Box<dyn Algorithm>> {
    match algorithm_name {
        "v1_0" => Ok(Box::new(Algorithm_v1_0::init(Hyperparameters_v1_0::default()))),
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

impl SimulatorPrices {
    /// Builds the moving averages once per date from `datetime_start` on, since closes only change at midnight
    fn init(prices_closing: &[PriceRow], prices_intraday: Vec<PriceRow>, datetime_start: NaiveDateTime) -> Self {
        let mut ma_4y_by_date: HashMap<NaiveDate, Timeseries<(f64, f64, f64)>> = HashMap::new();
        for price_row in &prices_intraday {
            let date = price_row.timestamp.date();
            if price_row.timestamp < datetime_start || ma_4y_by_date.contains_key(&date) {
                continue;
            }
            match get_ma_4y_datetime(prices_closing, price_row.timestamp) {
                Ok(ma_4y) => {
                    ma_4y_by_date.insert(date, ma_4y);
                }
                Err(e) => log::warn!("Skipping {}: {}", date, e),
            }
        }
        SimulatorPrices { prices_intraday, ma_4y_by_date }
    }
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

/// Purchases are appended in time order, so the scan stops at the first one older than 7 days
fn get_purchases_7d_datetime(
    purchases: &Timeseries<(Amount, Amount, f64)>,
    datetime_now: NaiveDateTime,
) -> Timeseries<(Amount, Amount, f64)> {
    let datetime_start = datetime_now - Duration::days(7);
    let mut purchases_7d: Timeseries<(Amount, Amount, f64)> = Timeseries::new(purchases.columns.clone());
    for purchase_row in purchases.data.iter().rev() {
        if purchase_row.timestamp < datetime_start {
            break;
        }
        if purchase_row.timestamp > datetime_now {
            continue;
        }
        purchases_7d.data.push(purchase_row.clone());
    }
    purchases_7d.data.reverse();
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

/// Replays every intraday price row from `datetime_start` to `datetime_end` as one timestep, adding
/// `deposit_initial` to the balance on the first timestep and `deposit_1d_simulated` on the first timestep of each
/// day. The algorithm still receives `params.deposit_1d` as its spend rate reference. Buys fill at `price_buy`;
/// sells are not simulated, matching the daemon. `pct_buy` is only computed for the steps when `is_pct_buy_saved`
async fn run_simulation(
    algorithm: &dyn Algorithm,
    prices: &SimulatorPrices,
    datetime_start: NaiveDateTime,
    datetime_end: NaiveDateTime,
    params: &AlgorithmParams,
    deposit_initial: &Amount,
    deposit_1d_simulated: &Amount,
    is_pct_buy_saved: bool,
) -> anyhow::Result<SimulatorResult> {
    let prices_intraday = &prices.prices_intraday;

    let mut balance = Amount::init(currency_market, 0.0);
    let mut amount_held = Amount::init(coin, 0.0);
    let mut amount_held_dca = Amount::init(coin, 0.0);
    let mut amount_deposited = Amount::init(currency_market, 0.0);
    let mut amount_spent = Amount::init(currency_market, 0.0);
    let mut amount_spent_first_half = Amount::init(currency_market, 0.0);
    let mut purchases: Timeseries<(Amount, Amount, f64)> = Timeseries::new(vec![
        "amount_out".to_string(),
        "amount_in".to_string(),
        "price".to_string(),
    ]);
    let mut steps: Vec<SimulatorStep> = Vec::new();
    let mut date_deposit_last: Option<NaiveDate> = None;
    let mut is_deposited_initial = false;
    let mut price_row_last: Option<&PriceRow> = None;
    let mut price_buy_sum = 0.0;
    let mut n_steps_below_reserve: usize = 0;

    for index_now in 0..prices_intraday.len() {
        let price_row = &prices_intraday[index_now];
        let datetime_now = price_row.timestamp;
        if datetime_now < datetime_start {
            continue;
        }
        if datetime_now > datetime_end {
            break;
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

        let ma_4y_now = match prices.ma_4y_by_date.get(&date_now) {
            Some(ma_4y_now) => ma_4y_now.clone(),
            None => continue,
        };
        price_row_last = Some(price_row);
        price_buy_sum += price_row.price_buy;

        let prices_30d = get_prices_30d_index(prices_intraday, index_now);
        let mut pct_buy: Option<f64> = None;
        if is_pct_buy_saved {
            pct_buy = get_pct_buy_step(&ma_4y_now, &prices_30d, price_row);
        }
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

        let params_now = AlgorithmParams {
            deposit_1d: params.deposit_1d.clone(),
            amount_buy_min: params.amount_buy_min.clone(),
            balance_reserve: params.balance_reserve.clone(),
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
        let command = match algorithm.run_algorithm(params_now, input).await {
            Ok(command) => Some(command),
            Err(e) => {
                log::warn!("Algorithm error at {}: {}", datetime_now, e);
                None
            }
        };
        let amount = match command {
            Some(command) => match (command.command, command.amount) {
                (1, Some(amount)) => Some(amount),
                _ => None,
            },
            None => None,
        };

        // Same 2 decimal floor as the daemon, so the order never overspends
        let amount = match amount {
            Some(amount) => Amount::init(&amount.currency, (amount.amount * 100.0).floor() / 100.0),
            None => Amount::init(currency_market, 0.0),
        };
        let mut is_bought = amount.amount > 0.0;
        if is_bought && amount.currency != balance.currency {
            log::warn!("Buy of {} is not in the balance currency at {}", amount, datetime_now);
            is_bought = false;
        }
        if is_bought && amount.amount > balance.amount {
            log::warn!("Buy of {:.2} exceeds balance {:.2} at {}", amount, balance, datetime_now);
            is_bought = false;
        }

        if is_bought {
            let amount_in = Amount::init(coin, amount.amount / price_row.price_buy);
            balance.amount -= amount.amount;
            amount_held.amount += amount_in.amount;
            amount_spent.amount += amount.amount;
            if datetime_now.hour() < 12 {
                amount_spent_first_half.amount += amount.amount;
            }
            let step_last = steps.last_mut().expect("A step is pushed before every buy");
            step_last.amount = amount.clone();
            purchases.data.push(TimeseriesRow {
                timestamp: datetime_now,
                value: (amount, amount_in, price_row.price_buy),
            });
        }
        if balance.amount < params.balance_reserve.amount {
            n_steps_below_reserve += 1;
        }
    }

    let price_row_last = match price_row_last {
        Some(price_row_last) => price_row_last,
        None => return Err(anyhow!("No intraday prices from {} to {}", datetime_start, datetime_end)),
    };
    let price_buy_mean = price_buy_sum / steps.len() as f64;

    Ok(SimulatorResult {
        steps,
        datetime_start,
        datetime_end,
        amount_deposited,
        amount_spent,
        amount_spent_first_half,
        balance,
        balance_reserve: params.balance_reserve.clone(),
        amount_held,
        amount_held_dca,
        price_sell_final: price_row_last.price_sell,
        price_buy_mean,
        n_purchases: purchases.len(),
        n_steps_below_reserve,
    })
}

fn get_simulator_metrics(result: &SimulatorResult) -> SimulatorMetrics {
    let deposited = result.amount_deposited.amount;
    let value_total = result.balance.amount + result.amount_held.amount * result.price_sell_final;
    let value_dca = result.amount_held_dca.amount * result.price_sell_final;
    let amount_held_neutral = result.amount_held.amount + result.balance.amount / result.price_buy_mean;
    let value_neutral = amount_held_neutral * result.price_sell_final;

    let mut prop_spent_first_half = 0.0;
    if result.amount_spent.amount > 0.0 {
        prop_spent_first_half = result.amount_spent_first_half.amount / result.amount_spent.amount;
    }
    let prop_steps_below_reserve = result.n_steps_below_reserve as f64 / result.steps.len() as f64;

    SimulatorMetrics {
        roi: value_total / deposited - 1.0,
        roi_dca: value_dca / deposited - 1.0,
        roi_neutral: value_neutral / deposited - 1.0,
        prop_steps_below_reserve,
        prop_spent_first_half,
    }
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
    let metrics = get_simulator_metrics(result);
    let value_held = Amount::init(currency, result.amount_held.amount * result.price_sell_final);
    let value_total = Amount::init(currency, result.balance.amount + value_held.amount);
    let profit = Amount::init(currency, value_total.amount - result.amount_deposited.amount);

    let mut price_average = Amount::init(currency, 0.0);
    if result.amount_held.amount > 0.0 {
        price_average.amount = result.amount_spent.amount / result.amount_held.amount;
    }
    let price_sell_final = Amount::init(currency, result.price_sell_final);

    log::info!("Simulation from {} to {}", result.datetime_start, result.datetime_end);
    log::info!("Deposited {:.2}, spent {:.2} in {} purchase(s)", result.amount_deposited, result.amount_spent, result.n_purchases);
    log::info!("Holding {:.8} at an average {:.2}, final sell price {:.2}", result.amount_held, price_average, price_sell_final);
    let value_dca = Amount::init(currency, result.amount_held_dca.amount * result.price_sell_final);
    let mut price_average_dca = Amount::init(currency, 0.0);
    if result.amount_held_dca.amount > 0.0 {
        price_average_dca.amount = result.amount_deposited.amount / result.amount_held_dca.amount;
    }
    let value_vs_dca = Amount::init(currency, value_total.amount - value_dca.amount);
    log::info!("DCA baseline: {:.8} at an average {:.2}, final value {:.2}, algorithm minus DCA {:.2}", result.amount_held_dca, price_average_dca, value_dca, value_vs_dca);
    log::info!(
        "Spent {:.1}% in the first half of each UTC day; balance below the {:.2} reserve for {:.1}% of timesteps",
        100.0 * metrics.prop_spent_first_half,
        result.balance_reserve,
        100.0 * metrics.prop_steps_below_reserve
    );
    log::info!(
        "Final value {:.2} (balance {:.2} + held {:.2}), profit {:.2} ({:.2}%, {:.2}% with the balance bought at the mean price)",
        value_total,
        result.balance,
        value_held,
        profit,
        100.0 * metrics.roi,
        100.0 * metrics.roi_neutral
    );
}

/// Every combination of the grid's values, each starting from the v1_0 defaults
fn get_sweep_cases_product<Value>(
    cases: Vec<SweepCase>,
    values: &[Value],
    set_value: impl Fn(&mut SweepCase, &Value),
) -> Vec<SweepCase> {
    let mut cases_product: Vec<SweepCase> = Vec::new();
    for case in &cases {
        for value in values {
            let mut case_product = case.clone();
            set_value(&mut case_product, value);
            cases_product.push(case_product);
        }
    }
    cases_product
}

fn get_sweep_cases(grid: &SweepGrid) -> Vec<SweepCase> {
    let case_default = SweepCase { balance_reserve: 0.0, hyperparameters: Hyperparameters_v1_0::default() };
    let mut cases: Vec<SweepCase> = vec![case_default];
    cases = get_sweep_cases_product(cases, &grid.balance_reserve, |case, value| case.balance_reserve = *value);
    cases = get_sweep_cases_product(cases, &grid.multiplier_max, |case, value| case.hyperparameters.multiplier_max = *value);
    cases = get_sweep_cases_product(cases, &grid.pct_neutral, |case, value| case.hyperparameters.pct_neutral = *value);
    cases = get_sweep_cases_product(cases, &grid.prop_balance_1d_max, |case, value| {
        case.hyperparameters.prop_balance_1d_max = *value
    });
    cases = get_sweep_cases_product(cases, &grid.prop_budget_step_max, |case, value| {
        case.hyperparameters.prop_budget_step_max = *value
    });
    cases = get_sweep_cases_product(cases, &grid.pct_reserve_unlock, |case, value| {
        case.hyperparameters.pct_reserve_unlock = *value
    });
    cases = get_sweep_cases_product(cases, &grid.days_runway, |case, value| case.hyperparameters.days_runway = *value);
    cases
}

/// Windows of `n_days_window` starting every `n_days_window_step` days, all ending by `datetime_last`
fn get_sweep_windows(grid: &SweepGrid, datetime_last: NaiveDateTime) -> Vec<(NaiveDateTime, NaiveDateTime)> {
    let mut windows: Vec<(NaiveDateTime, NaiveDateTime)> = Vec::new();
    let mut datetime_start = datetime_last - Duration::days(grid.n_days_last);
    loop {
        let datetime_end = datetime_start + Duration::days(grid.n_days_window);
        if datetime_end > datetime_last {
            break;
        }
        windows.push((datetime_start, datetime_end));
        datetime_start += Duration::days(grid.n_days_window_step);
    }
    windows
}

fn get_sweep_line(case: &SweepCase, result: &SimulatorResult) -> String {
    let metrics = get_simulator_metrics(result);
    let hyperparameters = &case.hyperparameters;
    format!(
        "{},{},{},{},{},{},{},{},{:.2},{:.2},{:.2},{:.6},{:.6},{:.6},{:.4},{:.4},{}",
        case.balance_reserve,
        hyperparameters.multiplier_max,
        hyperparameters.pct_neutral,
        hyperparameters.prop_balance_1d_max,
        hyperparameters.prop_budget_step_max,
        hyperparameters.pct_reserve_unlock,
        hyperparameters.days_runway,
        result.datetime_start,
        result.amount_deposited.amount,
        result.amount_spent.amount,
        result.balance.amount,
        metrics.roi,
        metrics.roi_dca,
        metrics.roi_neutral,
        metrics.prop_steps_below_reserve,
        metrics.prop_spent_first_half,
        result.n_purchases
    )
}

/// Runs `jobs` on one thread per core, each with its own single-threaded runtime, and returns one CSV line per
/// job in job order. A failed job logs and leaves its line out
fn run_sweep_jobs(
    grid: &SweepGrid,
    cases: &[SweepCase],
    jobs: &[SweepJob],
    prices: &SimulatorPrices,
) -> anyhow::Result<Vec<String>> {
    let n_threads = std::thread::available_parallelism()?.get();
    let index_job_next = std::sync::atomic::AtomicUsize::new(0);
    let lines: std::sync::Mutex<Vec<Option<String>>> = std::sync::Mutex::new(vec![None; jobs.len()]);
    let amount_buy_min = match grid.amount_buy_min {
        Some(amount_buy_min) => Some(Amount::init(currency_market, amount_buy_min)),
        None => None,
    };
    let deposit_1d = Amount::init(currency_market, grid.deposit_1d);
    let deposit_initial = Amount::init(currency_market, 0.0);

    std::thread::scope(|scope| {
        for _ in 0..n_threads {
            scope.spawn(|| {
                let runtime = match tokio::runtime::Builder::new_current_thread().build() {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        log::error!("Failed to build a sweep runtime: {}", e);
                        return;
                    }
                };
                loop {
                    let index_job = index_job_next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if index_job >= jobs.len() {
                        break;
                    }
                    let job = &jobs[index_job];
                    let case = &cases[job.index_case];
                    let algorithm = Algorithm_v1_0::init(case.hyperparameters.clone());
                    let params = AlgorithmParams {
                        deposit_1d: deposit_1d.clone(),
                        amount_buy_min: amount_buy_min.clone(),
                        balance_reserve: Amount::init(currency_market, case.balance_reserve),
                    };
                    let simulation = run_simulation(
                        &algorithm,
                        prices,
                        job.datetime_start,
                        job.datetime_end,
                        &params,
                        &deposit_initial,
                        &deposit_1d,
                        false,
                    );
                    let result = match runtime.block_on(simulation) {
                        Ok(result) => result,
                        Err(e) => {
                            log::warn!("Sweep job {} failed: {}", index_job, e);
                            continue;
                        }
                    };
                    let line = get_sweep_line(case, &result);
                    match lines.lock() {
                        Ok(mut lines) => lines[index_job] = Some(line),
                        Err(e) => log::error!("Lock on sweep lines poisoned: {}", e),
                    }
                    if index_job % 100 == 0 {
                        log::info!("Sweep job {}/{}", index_job, jobs.len());
                    }
                }
            });
        }
    });

    let lines = match lines.into_inner() {
        Ok(lines) => lines,
        Err(e) => return Err(anyhow!("Lock on sweep lines poisoned: {}", e)),
    };
    let mut lines_done: Vec<String> = Vec::new();
    for line in lines {
        if let Some(line) = line {
            lines_done.push(line);
        }
    }
    Ok(lines_done)
}

/// Backtests every combination in the grid at `grid_toml_path` on every window, starting each window from an
/// empty balance, and writes one row per combination and window to `results_csv_path`
fn run_sweep(config: &Config, grid_toml_path: &str, results_csv_path: &str) -> anyhow::Result<()> {
    let grid_str = std::fs::read_to_string(grid_toml_path)?;
    let grid: SweepGrid = toml::from_str(&grid_str)?;
    if grid.n_days_window <= 0 || grid.n_days_window_step <= 0 || grid.n_days_last < grid.n_days_window {
        return Err(anyhow!(
            "Invalid windows: 'n_days_window' {} and 'n_days_window_step' {} must be positive and fit in 'n_days_last' {}",
            grid.n_days_window,
            grid.n_days_window_step,
            grid.n_days_last
        ));
    }

    let database = Database::init(config.clone())?;
    let days_closing = grid.n_days_last as i32 + days_history_closing;
    let days_intraday = grid.n_days_last as i32 + days_history_intraday;
    let prices_closing = database.get_prices_last(coin, days_closing, true)?;
    let prices_intraday = database.get_prices_last(coin, days_intraday, false)?;
    let datetime_last = match prices_intraday.last() {
        Some(price_row_last) => price_row_last.timestamp,
        None => return Err(anyhow!("No intraday prices in the database")),
    };
    let datetime_first = datetime_last - Duration::days(grid.n_days_last);
    let prices = SimulatorPrices::init(&prices_closing, prices_intraday, datetime_first);

    let cases = get_sweep_cases(&grid);
    let windows = get_sweep_windows(&grid, datetime_last);
    let mut jobs: Vec<SweepJob> = Vec::new();
    for index_case in 0..cases.len() {
        for (datetime_start, datetime_end) in &windows {
            jobs.push(SweepJob { index_case, datetime_start: *datetime_start, datetime_end: *datetime_end });
        }
    }
    log::info!("Sweeping {} case(s) over {} window(s), {} job(s)", cases.len(), windows.len(), jobs.len());

    let lines_job = run_sweep_jobs(&grid, &cases, &jobs, &prices)?;
    let header = "balance_reserve,multiplier_max,pct_neutral,prop_balance_1d_max,prop_budget_step_max,pct_reserve_unlock,days_runway,datetime_start,deposited,spent,balance,roi,roi_dca,roi_neutral,prop_steps_below_reserve,prop_spent_first_half,n_purchases";
    let mut lines: Vec<String> = vec![header.to_string()];
    lines.extend(lines_job);
    std::fs::write(results_csv_path, lines.join("\n"))?;
    log::info!("Saved {} row(s) to {}", lines.len() - 1, results_csv_path);
    Ok(())
}

async fn run_simulator() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let config = Config::init();

    if args.get(1).map(String::as_str) == Some("sweep") {
        let (grid_toml_path, results_csv_path) = match (args.get(2), args.get(3)) {
            (Some(grid_toml_path), Some(results_csv_path)) => (grid_toml_path, results_csv_path),
            _ => return Err(anyhow!("Usage: simulator sweep <grid_toml_path> <results_csv_path>")),
        };
        return run_sweep(&config, grid_toml_path, results_csv_path);
    }

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
    // Missing means no reserve, matching the daemon
    let balance_reserve: f64 = match args.get(6) {
        Some(balance_reserve_str) => balance_reserve_str
            .parse()
            .map_err(|e| anyhow!("Invalid balance_reserve '{}': {}", balance_reserve_str, e))?,
        None => config.get_config("balance_reserve")?.unwrap_or(0.0),
    };
    let params = AlgorithmParams {
        deposit_1d: Amount::init(currency_market, deposit_1d),
        amount_buy_min,
        balance_reserve: Amount::init(currency_market, balance_reserve),
    };
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
    let datetime_end = match prices_intraday.last() {
        Some(price_row_last) => price_row_last.timestamp,
        None => return Err(anyhow!("No intraday prices in the database")),
    };
    let datetime_start = datetime_end - Duration::days(n_days_last);
    let prices = SimulatorPrices::init(&prices_closing, prices_intraday, datetime_start);

    let path_csv = args.get(3);
    let result = run_simulation(
        algorithm.as_ref(),
        &prices,
        datetime_start,
        datetime_end,
        &params,
        &deposit_initial,
        &deposit_1d_simulated,
        path_csv.is_some(),
    )
    .await?;
    log_simulator_result(&result);
    if let Some(path_csv) = path_csv {
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
