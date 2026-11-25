use crate::algorithm::v1_0::Algorithm_Buy_v1_0;
use crate::algorithm::Algorithm;
use crate::daemon::Daemon;
use crate::quant::moving_average::get_timeseries_ma_trailing;
use crate::structure::algorithm::{AlgorithmInput, AlgorithmParams};

use chrono::Local;

impl Daemon {
    async fn get_balance_aud(&self) -> anyhow::Result<f64> {
        let balance = self.client_cex.get_balance().await?;
        for item in balance {
            if item.currency.eq_ignore_ascii_case("aud") {
                return Ok(item.value);
            }
        }
        Err(anyhow::anyhow!("No AUD in CEx balance"))
    }

    /// Returns the (price_buy, price_sell) of `coin` from the CEx
    async fn get_prices_coin(&self, coin: &str) -> anyhow::Result<(f64, f64)> {
        let price_info = match self.client_cex.get_price_coin(coin).await? {
            Some(price_info) => price_info,
            None => return Err(anyhow::anyhow!("Price info not found for '{}'", coin)),
        };

        let price_buy_str = match price_info["ask"].as_str() {
            Some(price_buy_str) => price_buy_str,
            None => return Err(anyhow::anyhow!("Missing 'ask' in price info for '{}'", coin)),
        };
        let price_sell_str = match price_info["bid"].as_str() {
            Some(price_sell_str) => price_sell_str,
            None => return Err(anyhow::anyhow!("Missing 'bid' in price info for '{}'", coin)),
        };
        let price_buy: f64 = price_buy_str.parse()?;
        let price_sell: f64 = price_sell_str.parse()?;
        Ok((price_buy, price_sell))
    }

    /// Buys `amount_aud` of `coin` near the quoted rate, then records the fill in the purchases table
    async fn run_order_buy(&self, coin: &str, amount_aud: f64) -> anyhow::Result<()> {
        let rate = self
            .client_cex
            .get_quote_coin_buy(coin, amount_aud, "aud")
            .await?;
        let order = self
            .client_cex
            .push_order_buy_now(coin, amount_aud, "aud", rate)
            .await?;
        if order.amount <= 0.0 {
            return Err(anyhow::anyhow!(
                "Buy order returned no {}: {:?}",
                coin,
                order
            ));
        }

        let price = order.total / order.amount;
        log::info!(
            "Bought {} {} for {:.2} AUD at {} (quoted {}, market {})",
            order.amount,
            order.coin,
            order.total,
            price,
            rate,
            order.market
        );

        // The order has filled, so a failed save must be reconciled by hand
        match self.database.save_purchase(order.total, order.amount, price) {
            Ok(()) => Ok(()),
            Err(e) => Err(anyhow::anyhow!(
                "Bought {} {} for {:.2} AUD but failed to save the purchase: {}",
                order.amount,
                order.coin,
                order.total,
                e
            )),
        }
    }

    /// Places the algorithm's order when `is_wet_run`, otherwise only logs it
    async fn run_algorithm(&self, algorithm: &dyn Algorithm) -> anyhow::Result<()> {
        let balance = self.get_balance_aud().await?;
        let (price_buy, price_sell) = self.get_prices_coin("btc").await?;

        let prices_last = self.database.get_prices_last("btc", 365 * 8 + 3, true)?;
        let ma_4y = get_timeseries_ma_trailing(prices_last, "price_buy", 365 * 4 + 1)?;
        let purchases_7d = self.database.get_purchases_last(7)?;

        let params = AlgorithmParams {
            deposit_7d: self.deposit_7d,
            amount_buy_min: self.amount_buy_min,
            timestep_mins: self.timestep_mins,
        };
        let input = AlgorithmInput {
            datetime_now: Local::now().naive_local(),
            balance,
            price_buy,
            price_sell,
            ma_4y,
            purchases_7d,
        };

        let command = algorithm.run_algorithm(params, input).await?;
        let amount = match (command.command, command.amount) {
            (1, Some(amount)) => amount,
            _ => {
                log::info!("Idle at {} (balance {:.2} AUD)", price_buy, balance);
                return Ok(());
            }
        };

        // CoinSpot accepts at most 2 decimal places for AUD amounts; floor so we never overspend
        let amount_aud = (amount * 100.0).floor() / 100.0;
        if amount_aud <= 0.0 {
            log::info!("Idle: buy of {} AUD rounds to 0.00", amount);
            return Ok(());
        }
        if amount_aud > balance {
            return Err(anyhow::anyhow!(
                "Buy of {:.2} AUD exceeds balance {:.2} AUD",
                amount_aud,
                balance
            ));
        }

        if !self.is_wet_run {
            log::info!(
                "Dry run: would buy {:.2} AUD of BTC at {} (balance {:.2} AUD)",
                amount_aud,
                price_buy,
                balance
            );
            return Ok(());
        }
        self.run_order_buy("btc", amount_aud).await
    }

    pub async fn loop_run_algorithm(&self) {
        let algorithm = Algorithm_Buy_v1_0 {};
        loop {
            match self.run_algorithm(&algorithm).await {
                Ok(()) => {}
                Err(e) => log::error!("Error running algorithm: {}", e),
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
