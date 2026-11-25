use crate::structure::timeseries::{PriceRow, Timeseries, TimeseriesRow};
use chrono::NaiveDateTime;

pub struct AlgorithmParams {
    pub deposit_1d: f64, // Amount deposited into CEx balance per day
    pub amount_buy_min: Option<f64>, // Minimum purchase amount per transaction
}

pub struct AlgorithmInput {
    pub datetime_now: NaiveDateTime, // Current local time, aligned with the price and purchase timestamps
    pub balance: f64, 
    pub price_buy: f64,
    pub price_sell: f64,
    pub ma_4y: Timeseries<(f64, f64, f64)>,
    pub prices_30d: Vec<PriceRow>, // Intraday prices for the last 30 days, up to datetime_now
    pub purchases_7d: Timeseries<(f64, f64, f64)>, 
}

pub struct AlgorithmCommand {
    pub command: i32, // 0=idle, 1=buy, 2=sell
    pub amount: Option<f64>, // in AUD
}