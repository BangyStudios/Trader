use crate::structure::common::Amount;
use crate::structure::timeseries::{PriceRow, Timeseries, TimeseriesRow};
use chrono::NaiveDateTime;

pub struct AlgorithmParams {
    pub deposit_1d: Amount, // Amount deposited into CEx balance per day
    pub amount_buy_min: Option<Amount>, // Minimum purchase amount per transaction
}

pub struct AlgorithmInput {
    pub datetime_now: NaiveDateTime, // Current local time, aligned with the price and purchase timestamps
    pub balance: Amount, // In the market currency
    pub price_buy: f64,
    pub price_sell: f64,
    pub ma_4y: Timeseries<(f64, f64, f64)>,
    pub prices_30d: Vec<PriceRow>, // Intraday prices for the last 30 days, up to datetime_now
    pub purchases_7d: Timeseries<(Amount, Amount, f64)>, // (amount_out, amount_in, price)
}

pub struct AlgorithmCommand {
    pub command: i32, // 0=idle, 1=buy, 2=sell
    pub amount: Option<Amount>, // In the market currency
}
