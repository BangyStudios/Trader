use crate::config::Config;
use crate::structure::timeseries::PriceRow;
use chrono::{DateTime, Duration, Local, NaiveDateTime, NaiveTime, Utc};
use mysql::prelude::*;
use mysql::*;

#[derive(Debug)]

pub struct Database {
    pool: Pool,
    currencies_supported: Vec<String>,
}

impl Database {
    /// Creates a new Database instance and initializes the `price_btc` table if it doesn't exist
    pub fn init(config: Config) -> anyhow::Result<Self> {
        // Construct MySQL connection URL from config
        let url = format!(
            "mysql://{}:{}@{}/{}",
            config.get_config("db_user").unwrap(),
            config.get_config("db_pass").unwrap(),
            config.get_config("db_host").unwrap(),
            config.get_config("db_name").unwrap()
        );

        // Create a connection pool
        let pool = Pool::new(Opts::from_url(&url)?)?;

        // Initialize table if not exists
        let mut conn = pool.get_conn()?;
        conn.query_drop(
            r"CREATE TABLE IF NOT EXISTS price_btc (
                id INT AUTO_INCREMENT PRIMARY KEY,
                price_buy DOUBLE NOT NULL,
                price_sell DOUBLE NOT NULL,
                price_last DOUBLE NOT NULL,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
        )?;

        // Purchases table to track BTC purchases in AUD
        conn.query_drop(
            r"CREATE TABLE IF NOT EXISTS purchases_btc (
                id INT AUTO_INCREMENT PRIMARY KEY,
                amount_aud DOUBLE NOT NULL,
                amount_btc DOUBLE NOT NULL,
                price DOUBLE NOT NULL,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
        )?;

        log::info!("Connected to database and ensured price_btc table exists.");

        let currencies_supported = vec!["btc".to_string()]; // Example currencies

        Ok(Database {
            pool,
            currencies_supported,
        })
    }

    pub fn get_prices_last(&self, currency: &str, days: i32, closing: bool) -> anyhow::Result<Vec<PriceRow>> {
        if !self.currencies_supported.contains(&currency.to_string()) {
            Err(anyhow::format_err!(
                "Currency {} not currently supported",
                currency
            ))?;
        }

        let query = if closing {
            format!(
                r"SELECT p.* FROM price_{currency} p
                  INNER JOIN (
                      SELECT DATE(timestamp) AS day, MAX(timestamp) AS max_timestamp
                      FROM price_{currency}
                      WHERE timestamp >= :datetime_min AND timestamp < :datetime_max
                      GROUP BY day
                  ) latest ON p.timestamp = latest.max_timestamp
                  ORDER BY p.timestamp ASC",
                currency = currency
            )
        } else {
            format!(
                r"SELECT * FROM price_{}
                  WHERE timestamp >= :datetime_min AND timestamp < :datetime_max
                  ORDER BY timestamp ASC",
                currency
            )
        };

        let datetime_min: NaiveDateTime = (Local::now() - Duration::days(days as i64))
            .with_time(NaiveTime::MIN)
            .unwrap()
            .naive_local();
        let datetime_max: NaiveDateTime = (Local::now())
            .with_time(NaiveTime::MIN)
            .unwrap()
            .naive_local();

        let mut conn = self.pool.get_conn()?;
        let prices: Vec<PriceRow> = conn.exec_map(
            &query,
            params! {
                "datetime_min" => datetime_min.to_string(),
                "datetime_max" => datetime_max.to_string()
            },
            |(id, price_buy, price_sell, price_last, timestamp)| PriceRow { id, price_buy, price_sell, price_last, timestamp },
        )?;

        Ok(prices)
    }

    /// Loads purchases in the last `n_days` days. Returns a vector of tuples (amount_aud, amount_btc, price)
    pub fn get_purchases_last(&self, n_days: i32) -> anyhow::Result<Vec<(f64, f64, f64)>> {
        let query = r"SELECT amount_aud, amount_btc, price FROM purchases_btc
            WHERE timestamp >= :datetime_min AND timestamp < :datetime_max
            ORDER BY timestamp ASC";

        let datetime_min: NaiveDateTime = (Local::now() - Duration::days(n_days as i64))
            .with_time(NaiveTime::MIN)
            .unwrap()
            .naive_local();
        let datetime_max: NaiveDateTime = (Local::now())
            .with_time(NaiveTime::MIN)
            .unwrap()
            .naive_local();

        let mut conn = self.pool.get_conn()?;
        let purchases: Vec<(f64, f64, f64)> = conn.exec_map(
            query,
            params! {
                "datetime_min" => datetime_min.to_string(),
                "datetime_max" => datetime_max.to_string()
            },
            |(amount_aud, amount_btc, price)| (amount_aud, amount_btc, price),
        )?;

        Ok(purchases)
    }

    /// Saves a purchase record to `purchases_btc`.
    pub fn save_purchase(
        &self,
        amount_aud: f64,
        amount_btc: f64,
        price: f64,
    ) -> anyhow::Result<()> {
        let query = r"INSERT INTO purchases_btc (amount_aud, amount_btc, price) VALUES (:amount_aud, :amount_btc, :price)";
        let mut conn = self.pool.get_conn()?;
        conn.exec_drop(
            query,
            params! {
                "amount_aud" => amount_aud,
                "amount_btc" => amount_btc,
                "price" => price,
            },
        )?;
        Ok(())
    }

    /// Logs the current BTC price to the database
    pub fn save_price(
        &self,
        currency: &str,
        price_buy: f64,
        price_sell: f64,
        price_last: f64,
    ) -> anyhow::Result<()> {
        if !self.currencies_supported.contains(&currency.to_string()) {
            Err(anyhow::format_err!(
                "Currency {} not currently supported",
                currency
            ))?;
        }

        let query = format!(
            r"INSERT INTO price_{} (price_buy, price_sell, price_last)
              VALUES (:price_buy, :price_sell, :price_last)",
            currency
        );

        let mut conn = self.pool.get_conn()?;
        conn.exec_drop(
            &query,
            params! {
                "price_buy" => price_buy,
                "price_sell" => price_sell,
                "price_last" => price_last,
            },
        )?;
        Ok(())
    }
}

// Example usage with your CoinSpot struct (you'd put this in main.rs or wherever you use it):
/*
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::new(); // Your config initialization
    let coinspot = api::coinspot::CoinSpot::init(config);
    let db = Database::new()?;

    // Get BTC price and log it
    if let Some(price_info) = coinspot.get_price_coin("btc").await? {
        if let Some(price) = price_info["last"].as_f64() {
            db.log_btc_price(price)?;
            println!("Logged BTC price: {}", price);
        }
    }

    Ok(())
}
*/
