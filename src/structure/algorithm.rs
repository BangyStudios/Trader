use crate::structure::timeseries::{Timeseries, TimeseriesRow};
use chrono::NaiveDateTime;

pub struct AlgorithmParams {
    pub deposit_7d: f64, // Amount deposited into CEx balance per week
    pub amount_buy_min: Option<f64>, // Minimum purchase amount per transaction
    pub timestep_mins: u32, // Minutes between algorithm runs
}

pub struct AlgorithmInput {
    pub datetime_now: NaiveDateTime, // Current local time, aligned with the price and purchase timestamps
    pub balance: f64, 
    pub price_buy: f64,
    pub price_sell: f64,
    pub ma_4y: Timeseries<(f64, f64, f64)>, 
    pub purchases_7d: Timeseries<(f64, f64, f64)>, 
}

pub struct AlgorithmCommand {
    pub command: i32, // 0=idle, 1=buy, 2=sell
    pub amount: Option<f64>, // in AUD
}