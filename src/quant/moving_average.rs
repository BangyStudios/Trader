use anyhow::anyhow;
use chrono::NaiveDateTime;
use serde::de::Unexpected::Bool;

use crate::structure::timeseries::{PriceRow, Timeseries, TimeseriesRow};

pub fn get_ma_trailing(window: &[f64]) -> f64 {
    let mut sum = 0.0;
    for index in 0..window.len() {
        sum += window[index];
    }
    return sum / window.len() as f64
}

pub fn get_timeseries_diff(A: Vec<f64>, B: Vec<f64>) -> Vec<f64> {
    assert!(A.len() == B.len(), "Vector lengths are not equal, A ({}) B ({})", A.len(), B.len());
    let diff: Vec<f64> = A.iter()
        .zip(B.iter())
        .map(|(a, b)| a - b)
        .collect();
    return diff;
}

pub fn get_timeseries_ma_trailing(price_rows: Vec<PriceRow>, column: &str, len_window: i32) -> anyhow::Result<Timeseries<(f64, f64, f64)>> {
    let len_window = len_window as usize;

    if len_window > price_rows.len() {
        return Err(anyhow!("Window length cannot exceed data length"))
    }

    let values: Vec<f64> = price_rows.iter().map(|row| match column {
        "price_buy" => row.price_buy,
        "price_sell" => row.price_sell,
        "price_last" => row.price_last,
        _ => 0.0,
    }).collect();

    let mut data: Vec<TimeseriesRow<(f64, f64, f64)>> = Vec::new();
    let mut index_target = len_window;
    let mut index_start;
    let mut index_end;

    while index_target < price_rows.len() {
        index_start = index_target - len_window;
        index_end = index_target;

        let price_ma = get_ma_trailing(&values[index_start..index_end]);
        let price_actual = values[index_target];

        data.push(TimeseriesRow {
            timestamp: price_rows[index_target].timestamp,
            value: (price_ma, price_actual, price_actual / price_ma)
        });

        index_target += 1;
    }

    let result = Timeseries { 
        columns: vec!["price_ma".to_string(), "price_actual".to_string(), "prop_actual_ma".to_string()], 
        data 
    };

    Ok(result)
}