use crate::algorithm::v1_0::Algorithm_v1_0;
use crate::algorithm::Algorithm;
use crate::daemon::Daemon;
use crate::quant::moving_average::get_timeseries_ma_trailing;
use crate::structure::algorithm::{AlgorithmInput, AlgorithmParams};

use chrono::Utc;

impl Daemon {
    async fn get_balance_aud(&self) -> anyhow::Result<f64> {
        let balance = self.client_cex.pull_balance().await?;
        for item in balance {
            if item.currency.eq_ignore_ascii_case("aud") {
                return Ok(item.value);
            }
        }
        Err(anyhow::anyhow!("No AUD in CEx balance"))
    }

    /// Returns the (price_buy, price_sell) of `coin` from the CEx
    async fn get_prices_coin(&self, coin: &str) -> anyhow::Result<(f64, f64)> {
        let price_info = match self.client_cex.pull_price_coin(coin).await? {
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

    /// Buys about `amount_aud` of `coin` on the order book at up to `prop_rate_above_ask` over the ask, then records
    /// the fill in the purchases table. An unfilled order is cancelled and records nothing, so the amount accrues
    /// for a later timestep. Order-book buys cost a ~0.1% fee, taken from the coin received, against ~2.4% over
    /// the ask for buy-now
    async fn run_order_buy(&self, coin: &str, amount_aud: f64) -> anyhow::Result<()> {
        let prop_rate_above_ask = 0.005;

        let (price_ask, _) = self.get_prices_coin(coin).await?;
        // CoinSpot takes 8 decimal places; round the rate up and the coin amount down so the cost never exceeds `amount_aud`
        let rate = (price_ask * (1.0 + prop_rate_above_ask) * 100.0).ceil() / 100.0;
        let amount_coin = (amount_aud / rate * 1e8).floor() / 1e8;
        if amount_coin <= 0.0 {
            return Err(anyhow::anyhow!("Buy of {:.2} AUD at {} rounds to 0 {}", amount_aud, rate, coin));
        }

        let order = self
            .client_cex
            .push_order_buy_market(coin, amount_coin, rate)
            .await?;
        if order.amount <= 0.0 {
            log::warn!("Buy order of {} {} at {} did not fill", amount_coin, coin, rate);
            return Ok(());
        }

        let price = order.total / order.amount;
        let prop_price_above_ask = price / price_ask - 1.0;
        log::info!(
            "Bought {} {} for {:.2} AUD at {} ({:+.2}% over ask {}, limit {}, market {})",
            order.amount,
            order.coin,
            order.total,
            price,
            100.0 * prop_price_above_ask,
            price_ask,
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
        let ma_4y = get_timeseries_ma_trailing(prices_last, "price_mid", 365 * 4 + 1)?;
        let prices_30d = self.database.get_prices_last("btc", 30, false)?;
        let datetime_now = Utc::now().naive_utc();
        let mut purchases_7d = self.database.get_purchases_last(7)?;
        // A purchase after now means the stored timestamps are not UTC; the spending caps depend on them, so refuse to buy
        if let Some(purchase_last) = purchases_7d.latest() {
            if purchase_last.timestamp > datetime_now {
                return Err(anyhow::anyhow!(
                    "Last purchase at {} is after now ({} UTC); check that 'purchases_btc' timestamps are UTC",
                    purchase_last.timestamp,
                    datetime_now
                ));
            }
        }
        // Dry runs ignore real purchases, so each run shows what a fresh start would buy
        if !self.is_wet_run {
            purchases_7d.data.clear();
        }

        let params = AlgorithmParams {
            deposit_1d: self.deposit_1d,
            amount_buy_min: self.amount_buy_min,
        };
        let input = AlgorithmInput {
            datetime_now,
            balance,
            price_buy,
            price_sell,
            ma_4y,
            prices_30d,
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
        let algorithm = Algorithm_v1_0 {};
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
