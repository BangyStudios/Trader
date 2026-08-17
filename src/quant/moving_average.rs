use anyhow::anyhow;
use chrono::NaiveDateTime;

use crate::structure::timeseries::{PriceRow, TimeseriesRow};

pub fn get_ma_trailing(window: &[f64]) -> f64 {
    let mut sum = 0.0;
    for index in 0..window.len() {
        sum += window[index];
    }
    return sum / window.len() as f64
}

pub fn get_timeseries_ma_trailing(data: Vec<PriceRow>, column: &str, len_window: i32) -> anyhow::Result<Vec<TimeseriesRow<(f64, f64, f64)>>> {
    let len_window = len_window as usize;

    if len_window > data.len() {
        return Err(anyhow!("Window length cannot exceed data length"))
    }

    let values: Vec<f64> = data.iter().map(|row| match column {
        "price_buy" => row.price_buy,
        "price_sell" => row.price_sell,
        "price_last" => row.price_last,
        _ => 0.0,
    }).collect();

    let mut result = Vec::new();
    let mut index_target = len_window;
    let mut index_start;
    let mut index_end;

    while index_target < data.len() {
        index_start = index_target - len_window;
        index_end = index_target;

        let price_ma = get_ma_trailing(&values[index_start..index_end]);
        let price_actual = values[index_target];

        result.push(TimeseriesRow {
            timestamp: data[index_target].timestamp,
            value: (price_ma, price_actual, price_actual / price_ma)
        });

        index_target += 1;
    }

    Ok(result)
}