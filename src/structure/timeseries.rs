use chrono::NaiveDateTime;
use std::fmt;

pub struct PriceRow {
    pub id: i32,
    pub price_buy: f64,
    pub price_sell: f64,
    pub price_last: f64,
    pub timestamp: NaiveDateTime
}

pub struct TimeseriesRow<Value> {
    pub timestamp: NaiveDateTime,
    pub value: Value
}

pub struct Timeseries<Value> {
    pub columns: Vec<String>,
    pub data: Vec<TimeseriesRow<Value>>
}

#[derive(Debug)]
pub enum TimeseriesError {
    ColumnMismatch { expected: usize, found: usize },
    UnknownColumn(String),
    OutOfOrderTimestamp,
}

impl fmt::Display for TimeseriesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeseriesError::ColumnMismatch { expected, found } => write!(
                f,
                "row has {} value(s) but timeseries has {} column(s)",
                found, expected
            ),
            TimeseriesError::UnknownColumn(name) => write!(f, "unknown column '{}'", name),
            TimeseriesError::OutOfOrderTimestamp => {
                write!(f, "timestamp is older than the last row in the series")
            }
        }
    }
}

impl std::error::Error for TimeseriesError {}

impl<Value> Timeseries<Value> {
    pub fn new(columns: Vec<String>) -> Self {
        Timeseries { columns, data: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn latest(&self) -> Option<&TimeseriesRow<Value>> {
        self.data.last()
    }

    pub fn range(&self, start: NaiveDateTime, end: NaiveDateTime) -> Vec<&TimeseriesRow<Value>> {
        self.data
            .iter()
            .filter(|row| row.timestamp >= start && row.timestamp <= end)
            .collect()
    }

    fn column_index(&self, name: &str) -> Result<usize, TimeseriesError> {
        self.columns
            .iter()
            .position(|c| c == name)
            .ok_or_else(|| TimeseriesError::UnknownColumn(name.to_string()))
    }
}

impl<Value: Clone> Timeseries<Vec<Value>> {
    /// Inserts a row, enforcing that it has exactly one value per column and
    /// that timestamps stay in non-decreasing order.
    pub fn insert(&mut self, timestamp: NaiveDateTime, values: Vec<Value>) -> Result<(), TimeseriesError> {
        if values.len() != self.columns.len() {
            return Err(TimeseriesError::ColumnMismatch {
                expected: self.columns.len(),
                found: values.len(),
            });
        }
        if let Some(last) = self.data.last() {
            if timestamp < last.timestamp {
                return Err(TimeseriesError::OutOfOrderTimestamp);
            }
        }
        self.data.push(TimeseriesRow { timestamp, value: values });
        Ok(())
    }

    /// Returns every value recorded for a given column, in timestamp order.
    pub fn column(&self, name: &str) -> Result<Vec<&Value>, TimeseriesError> {
        let index = self.column_index(name)?;
        Ok(self.data.iter().map(|row| &row.value[index]).collect())
    }

    /// Returns a single (column, value) lookup for a specific row.
    pub fn get(&self, row: usize, name: &str) -> Result<Option<&Value>, TimeseriesError> {
        let index = self.column_index(name)?;
        Ok(self.data.get(row).map(|row| &row.value[index]))
    }
}
