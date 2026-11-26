use crate::config::Config;
use crate::structure::common::Amount;
use crate::structure::timeseries::{PriceRow, Timeseries, TimeseriesRow};
use chrono::{Duration, NaiveDateTime, NaiveTime, Utc};
use mysql::prelude::*;
use mysql::*;

#[derive(Debug)]

pub struct Database {
    pool: Pool,
    currencies_supported: Vec<String>,
    currency_market: String, // Currency that all prices are in, the second half of each `price_<coin><market>` table
}

impl Database {
    /// Creates a new Database instance and initializes the price and `purchases` tables if they don't exist
    pub fn init(config: Config) -> anyhow::Result<Self> {
        let db_user: String = config
            .get_config("db_user")?
            .ok_or_else(|| anyhow::anyhow!("Missing 'db_user' in config"))?;
        let db_pass: String = config
            .get_config("db_pass")?
            .ok_or_else(|| anyhow::anyhow!("Missing 'db_pass' in config"))?;
        let db_host: String = config
            .get_config("db_host")?
            .ok_or_else(|| anyhow::anyhow!("Missing 'db_host' in config"))?;
        let db_name: String = config
            .get_config("db_name")?
            .ok_or_else(|| anyhow::anyhow!("Missing 'db_name' in config"))?;

        // Construct MySQL connection URL from config
        let url = format!("mysql://{}:{}@{}/{}", db_user, db_pass, db_host, db_name);

        // Create a connection pool
        let pool = Pool::new(Opts::from_url(&url)?)?;

        let currencies_supported = vec!["btc".to_string()]; // Example currencies
        let currency_market = "aud".to_string();
        let database = Database {
            pool,
            currencies_supported,
            currency_market,
        };

        // Initialize tables if not exists
        let mut conn = database.pool.get_conn()?;
        for currency in &database.currencies_supported {
            let table_price = database.get_table_price(currency)?;
            let query = format!(
                r"CREATE TABLE IF NOT EXISTS {} (
                    id INT AUTO_INCREMENT PRIMARY KEY,
                    price_buy DOUBLE NOT NULL,
                    price_sell DOUBLE NOT NULL,
                    price_last DOUBLE NOT NULL,
                    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
                )",
                table_price
            );
            conn.query_drop(query)?;
        }

        // Purchases of any currency pair: `amount_out` of `currency_out` spent for `amount_in` of `currency_in`
        conn.query_drop(
            r"CREATE TABLE IF NOT EXISTS purchases (
                id INT AUTO_INCREMENT PRIMARY KEY,
                amount_out DOUBLE NOT NULL,
                currency_out VARCHAR(16) NOT NULL,
                amount_in DOUBLE NOT NULL,
                currency_in VARCHAR(16) NOT NULL,
                price DOUBLE NOT NULL,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
        )?;

        log::info!("Connected to database and ensured the price and purchases tables exist.");
        Ok(database)
    }

    /// Price table of `currency` against the market currency, such as `price_btcaud`. `currency` is checked against
    /// `currencies_supported`, since it is interpolated into SQL
    fn get_table_price(&self, currency: &str) -> anyhow::Result<String> {
        if !self.currencies_supported.contains(&currency.to_string()) {
            return Err(anyhow::format_err!("Currency {} not currently supported", currency));
        }
        let table_price = format!("price_{}{}", currency, self.currency_market);
        Ok(table_price)
    }

    pub fn get_prices_last(&self, currency: &str, days: i32, closing: bool) -> anyhow::Result<Vec<PriceRow>> {
        let table_price = self.get_table_price(currency)?;

        let query = if closing {
            format!(
                r"SELECT p.* FROM {table_price} p
                  INNER JOIN (
                      SELECT DATE(timestamp) AS day, MAX(timestamp) AS max_timestamp
                      FROM {table_price}
                      WHERE timestamp >= :datetime_min AND timestamp < :datetime_max
                      GROUP BY day
                  ) latest ON p.timestamp = latest.max_timestamp
                  ORDER BY p.timestamp ASC",
                table_price = table_price
            )
        } else {
            format!(
                r"SELECT * FROM {}
                  WHERE timestamp >= :datetime_min AND timestamp < :datetime_max
                  ORDER BY timestamp ASC",
                table_price
            )
        };

        // Timestamps are stored in UTC, so days run midnight to midnight UTC
        let datetime_now: NaiveDateTime = Utc::now().naive_utc();
        let datetime_min: NaiveDateTime = (datetime_now - Duration::days(days as i64))
            .date()
            .and_time(NaiveTime::MIN);
        // Closing prices stop at midnight so today's partial day is excluded; intraday prices run up to now
        let mut datetime_max: NaiveDateTime = datetime_now;
        if closing {
            datetime_max = datetime_now.date().and_time(NaiveTime::MIN);
        }

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

    /// Loads purchases of `currency_in` paid in `currency_out` from `n_days` days ago onwards, as a Timeseries of
    /// (amount_out, amount_in, price) rows. There is no upper bound, so rows timestamped after now are returned for
    /// the caller to reject
    pub fn get_purchases_last(
        &self,
        currency_out: &str,
        currency_in: &str,
        n_days: i32,
    ) -> anyhow::Result<Timeseries<(Amount, Amount, f64)>> {
        let query = r"SELECT timestamp, amount_out, currency_out, amount_in, currency_in, price FROM purchases
            WHERE timestamp >= :datetime_min AND currency_out = :currency_out AND currency_in = :currency_in
            ORDER BY timestamp ASC";

        let datetime_now: NaiveDateTime = Utc::now().naive_utc();
        let datetime_min: NaiveDateTime = (datetime_now - Duration::days(n_days as i64))
            .date()
            .and_time(NaiveTime::MIN);

        let mut conn = self.pool.get_conn()?;
        let rows: Vec<(NaiveDateTime, f64, String, f64, String, f64)> = conn.exec_map(
            query,
            params! {
                "datetime_min" => datetime_min.to_string(),
                "currency_out" => currency_out.to_ascii_lowercase(),
                "currency_in" => currency_in.to_ascii_lowercase(),
            },
            |(timestamp, amount_out, currency_out, amount_in, currency_in, price)| {
                (timestamp, amount_out, currency_out, amount_in, currency_in, price)
            },
        )?;

        let columns = vec![
            "amount_out".to_string(),
            "amount_in".to_string(),
            "price".to_string(),
        ];
        let mut purchases = Timeseries::new(columns);
        for (timestamp, amount_out, currency_out, amount_in, currency_in, price) in rows {
            let amount_out = Amount::init(&currency_out, amount_out);
            let amount_in = Amount::init(&currency_in, amount_in);
            purchases.data.push(TimeseriesRow {
                timestamp,
                value: (amount_out, amount_in, price),
            });
        }

        Ok(purchases)
    }

    /// Saves a purchase record to `purchases`
    pub fn save_purchase(
        &self,
        amount_out: &Amount,
        amount_in: &Amount,
        price: f64,
    ) -> anyhow::Result<()> {
        // Written in UTC by the app, since the column default `CURRENT_TIMESTAMP` uses the database server's zone
        let query = r"INSERT INTO purchases (amount_out, currency_out, amount_in, currency_in, price, timestamp)
            VALUES (:amount_out, :currency_out, :amount_in, :currency_in, :price, :timestamp)";
        let timestamp: NaiveDateTime = Utc::now().naive_utc();
        let mut conn = self.pool.get_conn()?;
        conn.exec_drop(
            query,
            params! {
                "amount_out" => amount_out.amount,
                "currency_out" => &amount_out.currency,
                "amount_in" => amount_in.amount,
                "currency_in" => &amount_in.currency,
                "price" => price,
                "timestamp" => timestamp.to_string(),
            },
        )?;
        Ok(())
    }

    /// Logs the current price of `currency` to its price table
    pub fn save_price(
        &self,
        currency: &str,
        price_buy: f64,
        price_sell: f64,
        price_last: f64,
    ) -> anyhow::Result<()> {
        let table_price = self.get_table_price(currency)?;

        // Written in UTC by the app, since the column default `CURRENT_TIMESTAMP` uses the database server's zone
        let query = format!(
            r"INSERT INTO {} (price_buy, price_sell, price_last, timestamp)
              VALUES (:price_buy, :price_sell, :price_last, :timestamp)",
            table_price
        );
        let timestamp: NaiveDateTime = Utc::now().naive_utc();

        let mut conn = self.pool.get_conn()?;
        conn.exec_drop(
            &query,
            params! {
                "price_buy" => price_buy,
                "price_sell" => price_sell,
                "price_last" => price_last,
                "timestamp" => timestamp.to_string(),
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
    if let Some(price_info) = coinspot.pull_price_coin("btc").await? {
        if let Some(price) = price_info["last"].as_f64() {
            db.log_btc_price(price)?;
            println!("Logged BTC price: {}", price);
        }
    }

    Ok(())
}
*/
