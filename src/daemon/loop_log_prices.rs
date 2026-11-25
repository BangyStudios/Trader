use crate::daemon::Daemon;

impl Daemon {
    pub async fn loop_log_prices(&self) {
        loop {
            match self.client_cex.pull_price_coin("btc").await {
                Ok(Some(price_info)) => {
                    let parse_price = |key: &str| -> Option<f64> {
                        price_info[key].as_str()?.parse::<f64>().ok()
                    };

                    let price_last = parse_price("last");
                    let price_buy = parse_price("ask");
                    let price_sell = parse_price("bid");

                    match (price_last, price_buy, price_sell) {
                        (Some(price_last), Some(price_buy), Some(price_sell)) => {
                            if let Err(e) = self.database
                                .save_price("btc", price_buy, price_sell, price_last)
                            {
                                log::error!("Failed to log BTC prices: {}", e);
                            } else {
                                log::info!(
                                    "Logged BTC prices - Buy: {}, Sell: {}, Last: {}",
                                    price_buy,
                                    price_sell,
                                    price_last
                                );
                            }
                        }
                        _ => log::warn!("One or more BTC price fields are missing or invalid."),
                    }
                }
                Ok(None) => log::warn!("Price info not found for BTC."),
                Err(e) => log::error!("Error fetching BTC price: {}", e),
            }

            let time_sleep_seconds = self.get_seconds_to_timestep_next().await;
            log::info!(
                "Sleeping for {} seconds until next timestep.",
                time_sleep_seconds
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(time_sleep_seconds.into())).await;
        }
    }
}
