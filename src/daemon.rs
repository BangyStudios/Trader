use crate::api;
use crate::config;
use crate::database;

pub struct Daemon {
    config: config::Config, 
    database: database::Database, 
    client_cex: Box<dyn api::CExClient>, 
    timestep_mins: u64, 
}

impl Daemon {
    pub fn new(client_cex_name: &str) -> anyhow::Result<Self> {
        let config = config::Config::init();
        let database = database::Database::init()?;
        
        let client_cex = match client_cex_name { 
            "coinspot" => api::coinspot::init_cex_client(&config)?,
            _ => return Err(anyhow::format_err!("Unsupported CEx client: {}", client_cex_name)),
        };

        let timestep_mins = config
            .get_config("timestep_mins")
            .unwrap_or("5")
            .parse::<u64>()
            .unwrap_or(5);

        return Ok(Daemon { 
            config, 
            database, 
            client_cex, 
            timestep_mins
        })
    }

    pub async fn run(&self) {
        self.loop_log_prices().await;
    }

    pub async fn loop_log_prices(&self) {
        loop {
            match self.client_cex.get_price_coin("btc").await {
                Ok(Some(price_info)) => {
                    println!("{}", price_info);
                    let parse_price = |key: &str| -> Option<f64> {
                        price_info[key].as_str()?.parse::<f64>().ok()
                    };

                    let price_last = parse_price("last");
                    let price_buy = parse_price("ask");
                    let price_sell = parse_price("bid");

                    match (price_last, price_buy, price_sell) {
                        (Some(price_last), Some(price_buy), Some(price_sell)) => {
                            if let Err(e) = self.database.log_price_btc(price_buy, price_sell, price_last) {
                                eprintln!("Failed to log BTC prices: {}", e);
                            } else {
                                println!("Logged BTC prices - Buy: {}, Sell: {}, Last: {}", price_buy, price_sell, price_last);
                            }
                        }
                        _ => eprintln!("One or more BTC price fields are missing or invalid."),
                    }
                }
                Ok(None) => eprintln!("Price info not found for BTC."),
                Err(e) => eprintln!("Error fetching BTC price: {}", e),
            }

            let coin = "BTC";
            let amount = 1.0;
            match self.client_cex.get_quote_coin_buy(&coin, &amount).await {
                Ok(Some(quote_info)) => {
                    println!("Quote info for {}: {}", &coin, &quote_info);
                }
                Ok(None) => {
                    println!("No quote info found for {}", &coin);
                }
                Err(e) => {
                    eprintln!("Error fetching quote info for {}: {}", &coin, e);
                }
            }

            // Wait for the configured timestep before the next iteration
            tokio::time::sleep(tokio::time::Duration::from_secs(self.timestep_mins * 60)).await;
        }
    }
}