use crate::config::Config;
use mysql::*;
use mysql::prelude::*;

pub struct Database {
    pool: Pool,
}

impl Database {
    /// Creates a new Database instance and initializes the `price_btc` table if it doesn't exist
    pub fn init(config: Config) -> anyhow::Result<Self> {
        // Construct MySQL connection URL from config
        let url = format!(
            "mysql://{}:{}@{}/{}",
            config.get_config("db_user").unwrap(), config.get_config("db_pass").unwrap(), config.get_config("db_host").unwrap(), config.get_config("db_name").unwrap()
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

        Ok(Database { pool })
    }

    /// Logs the current BTC price to the database
    pub fn log_price_btc(
        &self,
        price_buy: f64,
        price_sell: f64,
        price_last: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.pool.get_conn()?;
        conn.exec_drop(
            r"INSERT INTO price_btc (price_buy, price_sell, price_last)
              VALUES (:price_buy, :price_sell, :price_last)",
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