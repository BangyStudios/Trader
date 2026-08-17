use crate::structure::timeseries::TimeseriesRow;

pub struct AlgorithmParams {
    proportion: f64
}

pub struct AlgorithmInput {
    pub balance: f64, 
    pub price_buy: f64,
    pub price_sell: f64,
    pub price_history: Vec<TimeseriesRow<f64>>
}

pub struct AlgorithmCommand {
    pub command: String, // [buy, sell, idle]
    pub amount: Option<f64>, // in AUD
}