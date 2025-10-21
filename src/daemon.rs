use crate::api;
use crate::config;
use crate::database;

pub struct Daemon {
    config: config::Config, 
    database: database::Database, 
    client_cex: Box<dyn api::CExClient>,
}

impl Daemon {
    pub fn new(client_cex_name: &str) -> anyhow::Result<Self> {
        let config = config::Config::init();
        let database = database::Database::init()?;
        
        let client_cex = match client_cex_name { 
            "coinspot" => api::coinspot::init_cex_client(&config)?,
            _ => return Err(anyhow::format_err!("Unsupported CEx client: {}", client_cex_name)),
        };

        return Ok(Daemon { 
            config, 
            database, 
            client_cex,
        })
    }

    pub async fn run(&self) {
        self.log_btc_price_periodically().await;
    }

    pub async fn log_btc_price_periodically(&self) {
        loop {
            match self.client_cex.get_price_coin("btc").await {
                Ok(Some(price_info)) => {
                    if let Some(price_str) = price_info["last"].as_str() {
                        match price_str.parse::<f64>() {
                            Ok(price) => {
                                if let Err(e) = self.database.log_price_btc(price) {
                                    eprintln!("Failed to log BTC price: {}", e);
                                } else {
                                    println!("Logged BTC price: {}", price);
                                }
                            }
                            Err(e) => eprintln!("Failed to parse BTC price string: {}", e),
                        }
                    } else {
                        eprintln!("BTC price field missing or not a string.");
                    }
                }
                Ok(None) => eprintln!("Price info not found for BTC."),
                Err(e) => eprintln!("Error fetching BTC price: {}", e),
            }

            // Wait for 5 minutes before the next iteration
            tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;
        }
    }
}