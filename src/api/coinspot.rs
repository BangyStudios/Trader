// src/api/coinspot.rs
use crate::api::{CExBalance, CExBalanceItem, CExClient, CExOrderBuy};
use crate::config::Config;
use crate::structure::common::Amount;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use hmac::{Hmac, KeyInit, Mac};
use log;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha512;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn init_cex_client(config: &crate::config::Config) -> Result<Box<dyn CExClient>> {
    Ok(Box::new(CoinSpot::init(config)?))
}

pub struct CoinSpot {
    api_key: String,
    api_secret: String,
    client: reqwest::Client,
    config: Config,
    currency_market: String, // Currency that orders are paid in
    slippage_buy_max: f64, // Percent the fill rate may rise above the quoted rate before CoinSpot cancels the buy, in [0, 1000]
    order_fill_wait_secs: u64, // Wait before cancelling the unfilled rest of an order-book buy
}

/// CoinSpot checks `sign` against the body as JavaScript re-serializes it, which writes whole
/// floats without a fraction (`1`, not serde's `1.0`); a mismatch is "failed signature"
fn serialize_f64_js<Serializer: serde::Serializer>(
    value: &f64,
    serializer: Serializer,
) -> std::result::Result<Serializer::Ok, Serializer::Error> {
    // JavaScript's Number.MAX_SAFE_INTEGER
    let value_integer_max = 9007199254740991.0;
    let is_integer = value.fract() == 0.0 && value.abs() <= value_integer_max;
    if is_integer {
        return serializer.serialize_i64(*value as i64);
    }
    serializer.serialize_f64(*value)
}

#[derive(Serialize)]
struct CoinSpotRequestQuote<'a> {
    nonce: String,
    cointype: &'a str,
    #[serde(serialize_with = "serialize_f64_js")]
    amount: f64,
    amounttype: &'a str,
}

#[derive(Serialize)]
struct CoinSpotRequestOrderBuyNow<'a> {
    nonce: String,
    cointype: &'a str,
    amounttype: &'a str,
    #[serde(serialize_with = "serialize_f64_js")]
    amount: f64,
    #[serde(serialize_with = "serialize_f64_js")]
    rate: f64,
    #[serde(serialize_with = "serialize_f64_js")]
    threshold: f64,
    direction: &'a str,
}

#[derive(Deserialize)]
struct CoinSpotResponseOrderBuyNow {
    amount: f64, // Coin received
    total: f64,  // Paid, in the market currency
}

#[derive(Deserialize)]
struct CoinSpotResponseOrderBuy {
    id: String,
    coin: String,
    market: String,
}

#[derive(Deserialize)]
struct CoinSpotOrderOpen {
    id: String,
}

#[derive(Deserialize)]
struct CoinSpotResponseOrdersOpen {
    buyorders: Vec<CoinSpotOrderOpen>,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct CoinSpotOrderCompleted {
    id: String,
    amount: f64, // Coin received
    rate: f64,
    total: f64, // Market currency, excluding the fee
    audfeeExGst: Option<f64>,
    audGst: Option<f64>,
}

#[derive(Deserialize)]
struct CoinSpotResponseOrdersCompleted {
    buyorders: Vec<CoinSpotOrderCompleted>,
}

#[derive(Deserialize)]
struct CoinSpotResponseStatus {
    status: String,
    message: Option<String>,
}

#[derive(Deserialize)]
struct CoinSpotResponseQuote {
    rate: f64,
}

/// `value` rounded to `n_decimals` as a plain decimal without trailing zeros, which is how JavaScript writes
/// numbers in [1e-6, 1e21). serde writes small floats in exponent form (`1.682e-5`), which fails CoinSpot's
/// signature check
fn format_f64_js(value: f64, n_decimals: usize) -> String {
    let value_str = format!("{:.*}", n_decimals, value);
    if !value_str.contains('.') {
        return value_str;
    }
    let value_str = value_str.trim_end_matches('0');
    let value_str = value_str.trim_end_matches('.');
    value_str.to_string()
}

/// Milliseconds since the epoch; CoinSpot requires each nonce to exceed the previous one
fn get_nonce() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System clock is after the epoch")
        .as_millis()
        .to_string()
}

impl CoinSpot {
    pub fn init(config: &Config) -> Result<Self> {
        let client = reqwest::Client::new();

        let api_key: String = config
            .get_config("coinspot_api_key")?
            .ok_or_else(|| anyhow!("Missing 'coinspot_api_key' in config"))?;

        let api_secret: String = config
            .get_config("coinspot_api_secret")?
            .ok_or_else(|| anyhow!("Missing 'coinspot_api_secret' in config"))?;

        Ok(Self {
            api_key,
            api_secret,
            client,
            config: config.clone(),
            currency_market: "aud".to_string(),
            slippage_buy_max: 1.0,
            order_fill_wait_secs: 3,
        })
    }

    async fn is_order_buy_open(&self, cointype: &str, id: &str) -> anyhow::Result<bool> {
        let body = format!(r#"{{"nonce":"{}","cointype":"{}"}}"#, get_nonce(), cointype);
        let response_body = self
            .push_request_signed("https://www.coinspot.com.au/api/v2/ro/my/orders/market/open", &body)
            .await?;
        let response: CoinSpotResponseOrdersOpen = serde_json::from_str(&response_body)?;

        let mut is_open = false;
        for order in response.buyorders {
            if order.id == id {
                is_open = true;
            }
        }
        Ok(is_open)
    }

    /// CoinSpot only lodges the cancel; it fails if the order has started filling
    async fn push_order_buy_cancel(&self, id: &str) -> anyhow::Result<()> {
        let body = format!(r#"{{"nonce":"{}","id":"{}"}}"#, get_nonce(), id);
        self.push_request_signed("https://www.coinspot.com.au/api/v2/my/buy/cancel", &body)
            .await?;
        Ok(())
    }

    /// CoinSpot's `amounttype` for `amount`: "coin" when it is in `coin`, otherwise the market currency
    fn get_amounttype(&self, coin: &str, amount: &Amount) -> anyhow::Result<String> {
        if amount.currency.eq_ignore_ascii_case(coin) {
            return Ok("coin".to_string());
        }
        if amount.currency == self.currency_market {
            return Ok(self.currency_market.clone());
        }
        Err(anyhow!(
            "Amount in '{}' is neither coin '{}' nor market '{}'",
            amount.currency,
            coin,
            self.currency_market
        ))
    }

    /// Sums the completed rows of buy order `id` in the last 24 hours (the endpoint's default window) into
    /// (amount_in, amount_out). Both are zero when nothing filled.
    /// `startdate` is left out: the docs say seconds, but the endpoint silently returns no rows unless it is in milliseconds
    async fn pull_order_buy_fill(&self, cointype: &str, id: &str) -> anyhow::Result<(Amount, Amount)> {
        let body = format!(r#"{{"nonce":"{}","cointype":"{}"}}"#, get_nonce(), cointype);
        let response_body = self
            .push_request_signed("https://www.coinspot.com.au/api/v2/ro/my/orders/market/completed", &body)
            .await?;
        let response: CoinSpotResponseOrdersCompleted = match serde_json::from_str(&response_body) {
            Ok(response) => response,
            Err(e) => {
                return Err(anyhow!(
                    "Completed orders response for buy order {} is unparseable: {}, body: {}",
                    id,
                    e,
                    response_body
                ))
            }
        };

        let mut amount_in = Amount::init(cointype, 0.0);
        let mut amount_out = Amount::init(&self.currency_market, 0.0);
        for order in response.buyorders {
            if order.id != id {
                continue;
            }
            // The fee is taken from the coin received, so `amount` is net of it. `total` (ordered coin amount times
            // `rate`) is used over `audtotal`, which is rounded to cents and skews the price of small fills
            let amount_order = Amount::init(cointype, order.amount);
            let total_order = Amount::init(&self.currency_market, order.total);
            let fee = Amount::init(&self.currency_market, order.audfeeExGst.unwrap_or(0.0) + order.audGst.unwrap_or(0.0));
            log::info!(
                "Buy order {} filled {} at {} for {} (fee {} in coin)",
                id,
                amount_order,
                order.rate,
                total_order,
                fee
            );
            amount_in.amount += amount_order.amount;
            amount_out.amount += total_order.amount;
        }
        Ok((amount_in, amount_out))
    }

    /// Signs and POSTs `body`, returning the response body; errors unless the HTTP status
    /// is a success and the CoinSpot `status` is "ok"
    async fn push_request_signed(&self, url: &str, body: &str) -> anyhow::Result<String> {
        let sign = self.get_sign_body(body).await;

        let mut headers = HeaderMap::new();
        headers.insert("key", HeaderValue::from_str(&self.api_key)?);
        headers.insert("sign", HeaderValue::from_str(&sign)?);
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));

        let response = self
            .client
            .post(url)
            .headers(headers)
            .body(body.to_string())
            .send()
            .await?;
        let status_http = response.status();
        let response_body = response.text().await?;
        if !status_http.is_success() {
            return Err(anyhow!(
                "Request to '{}' failed with status {}, body: {}",
                url,
                status_http,
                response_body
            ));
        }

        let response_status: CoinSpotResponseStatus = serde_json::from_str(&response_body)?;
        if response_status.status != "ok" {
            let message = response_status.message.unwrap_or_default();
            return Err(anyhow!(
                "Request to '{}' returned status '{}': {}",
                url,
                response_status.status,
                message
            ));
        }
        Ok(response_body)
    }
}

#[async_trait]
impl CExClient for CoinSpot {
    fn print_api_key(&self) -> anyhow::Result<String> {
        // This is likely for debugging—log it if used, but avoid exposing secrets in prod!
        // Consider removing or gating behind a debug flag.
        Ok(format!(
            "API Key: {}, API Secret: {}",
            self.api_key, self.api_secret
        ))
    }

    async fn pull_prices(&self) -> anyhow::Result<serde_json::Value> {
        let mut headers = HeaderMap::new();
        headers.insert("key", HeaderValue::from_str(&self.api_key)?);
        headers.insert("sign", HeaderValue::from_str(&self.api_secret)?);

        let response = self
            .client
            .get("https://www.coinspot.com.au/pubapi/v2/latest")
            .headers(headers)
            .send()
            .await
            .map_err(|e| {
                log::error!("Request error: {}", e);
                e
            })?;

        if response.status().is_success() {
            let response_body = response.text().await.map_err(|e| {
                log::error!("Response error: {}", e);
                e
            })?;
            let response_json: Value = serde_json::from_str(&response_body).map_err(|e| {
                log::error!("JSON parsing error: {}", e);
                e
            })?;
            Ok(response_json)
        } else {
            log::error!("Request failed with status: {}", response.status());
            Err(anyhow::format_err!("Request failed"))
        }
    }

    async fn pull_price_coin(&self, coin: &str) -> anyhow::Result<Option<serde_json::Value>> {
        let json_value = self.pull_prices().await?;
        let price_info = json_value["prices"][coin].clone();
        Ok(if price_info.is_null() {
            None
        } else {
            Some(price_info)
        })
    }

    async fn get_sign_body(&self, body: &str) -> String {
        let mut mac = Hmac::<Sha512>::new_from_slice(self.api_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(body.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    async fn pull_quote_coin_buy(&self, coin: &str, amount: &Amount) -> anyhow::Result<f64> {
        let cointype = coin.to_uppercase();
        let amounttype = self.get_amounttype(coin, amount)?;
        let request = CoinSpotRequestQuote {
            nonce: get_nonce(),
            cointype: &cointype,
            amount: amount.amount,
            amounttype: &amounttype,
        };
        let body = serde_json::to_string(&request)?;
        let response_body = self
            .push_request_signed("https://www.coinspot.com.au/api/v2/quote/buy/now", &body)
            .await?;

        let response: CoinSpotResponseQuote = serde_json::from_str(&response_body)?;
        Ok(response.rate)
    }

    async fn push_order_buy_now(&self, coin: &str, amount: &Amount, rate: f64) -> anyhow::Result<CExOrderBuy> {
        let cointype = coin.to_uppercase();
        let amounttype = self.get_amounttype(coin, amount)?;
        let request = CoinSpotRequestOrderBuyNow {
            nonce: get_nonce(),
            cointype: &cointype,
            amounttype: &amounttype,
            amount: amount.amount,
            rate,
            threshold: self.slippage_buy_max,
            direction: "UP",
        };
        let body = serde_json::to_string(&request)?;
        let response_body = self
            .push_request_signed("https://www.coinspot.com.au/api/v2/my/buy/now", &body)
            .await?;

        // The order has filled by now, so keep the raw body for reconciling a parse failure
        let order: CoinSpotResponseOrderBuyNow = match serde_json::from_str(&response_body) {
            Ok(order) => order,
            Err(e) => {
                log::error!(
                    "Buy-now order filled but response is unparseable: {}, body: {}",
                    e,
                    response_body
                );
                return Err(e.into());
            }
        };
        Ok(CExOrderBuy {
            amount_in: Amount::init(coin, order.amount),
            amount_out: Amount::init(&self.currency_market, order.total),
        })
    }

    async fn push_order_buy_market(&self, amount: &Amount, rate: f64) -> anyhow::Result<CExOrderBuy> {
        let cointype = amount.currency.to_uppercase();
        let markettype = self.currency_market.to_uppercase();
        let body = format!(
            r#"{{"nonce":"{}","cointype":"{}","amount":{},"rate":{},"markettype":"{}"}}"#,
            get_nonce(),
            cointype,
            format_f64_js(amount.amount, 8),
            format_f64_js(rate, 8),
            markettype
        );
        let response_body = self
            .push_request_signed("https://www.coinspot.com.au/api/v2/my/buy", &body)
            .await?;
        // The order is on the book by now, so keep the raw body for reconciling a parse failure
        let order: CoinSpotResponseOrderBuy = match serde_json::from_str(&response_body) {
            Ok(order) => order,
            Err(e) => {
                return Err(anyhow!(
                    "Buy order placed but response is unparseable: {}, body: {}",
                    e,
                    response_body
                ))
            }
        };
        log::info!("Placed buy order {} for {} at {} (coin {}, market {})", order.id, amount, rate, order.coin, order.market);

        tokio::time::sleep(tokio::time::Duration::from_secs(self.order_fill_wait_secs)).await;
        let is_open = self.is_order_buy_open(&cointype, &order.id).await?;
        if is_open {
            log::warn!("Buy order {} still open after {} s, cancelling", order.id, self.order_fill_wait_secs);
            self.push_order_buy_cancel(&order.id).await?;
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            let is_open = self.is_order_buy_open(&cointype, &order.id).await?;
            if is_open {
                return Err(anyhow!("Buy order {} is still open after cancelling; reconcile by hand", order.id));
            }
        }

        let (amount_in, amount_out) = self.pull_order_buy_fill(&cointype, &order.id).await?;
        Ok(CExOrderBuy { amount_in, amount_out })
    }

    async fn pull_balance(&self) -> anyhow::Result<CExBalance> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_millis()
            .to_string();

        let body = format!(r#"{{"nonce":"{}"}}"#, nonce);
        let sign = self.get_sign_body(&body).await;

        let mut headers = HeaderMap::new();
        headers.insert("key", HeaderValue::from_str(&self.api_key)?);
        headers.insert("sign", HeaderValue::from_str(&sign)?);
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));

        // CoinSpot balance endpoint (private)
        let response = self
            .client
            .post("https://www.coinspot.com.au/api/ro/my/balances")
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|e| {
                log::error!("Request error: {}", e);
                e
            })?;

        if response.status().is_success() {
            let response_body = response.text().await.map_err(|e| {
                log::error!("Response error: {}", e);
                e
            })?;
            let response_json: Value = serde_json::from_str(&response_body).map_err(|e| {
                log::error!("JSON parsing error: {}", e);
                e
            })?;
            if response_json.get("status").and_then(|v| v.as_str()) == Some("ok") {
                let response_balances = response_json["balances"].clone();
                let balance = response_balances
                    .as_array()
                    .ok_or_else(|| anyhow::Error::msg("Expected an array"))
                    .and_then(|array| {
                        array.iter()
                        .map(|item| {
                            let (currency, data) = item
                                .as_object()
                                .and_then(|obj| obj.iter().next())
                                .ok_or_else(|| anyhow::Error::msg("empty balance item".to_string()))?;

                            let amount_fiat = data.get("audbalance").and_then(Value::as_f64)
                                .ok_or_else(|| anyhow::Error::msg("missing audbalance".to_string()))?;
                            let amount = data.get("balance").and_then(Value::as_f64)
                                .ok_or_else(|| anyhow::Error::msg("missing balance".to_string()))?;
                            let rate = data.get("rate").and_then(Value::as_f64)
                                .ok_or_else(|| anyhow::Error::msg("missing rate".to_string()))?;

                            Ok(CExBalanceItem {
                                amount: Amount::init(currency, amount),
                                amount_fiat: Amount::init(&self.currency_market, amount_fiat),
                                rate,
                            })
                        })
                        .collect::<Result<CExBalance, anyhow::Error>>()
                    })?;

                Ok(balance)
            } else {
                Err(anyhow::format_err!(
                    "Response does not contain expected 'balances' field, response: {:?}",
                    response_json
                ))
            }
        } else {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            log::error!(
                "Balance request failed with status: {}, body: {}",
                status,
                body_text
            );
            Err(anyhow::format_err!("Request failed"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_f64_js_matches_javascript() {
        let request = CoinSpotRequestOrderBuyNow {
            nonce: "1".to_string(),
            cointype: "BTC",
            amounttype: "aud",
            amount: 1.0,
            rate: 150234.57,
            threshold: 1.0,
            direction: "UP",
        };
        let body = serde_json::to_string(&request).expect("Request serializes");
        assert_eq!(
            body,
            r#"{"nonce":"1","cointype":"BTC","amounttype":"aud","amount":1,"rate":150234.57,"threshold":1,"direction":"UP"}"#
        );
    }

    #[test]
    fn test_format_f64_js_writes_plain_decimals() {
        assert_eq!(format_f64_js(0.00001682, 8), "0.00001682");
        assert_eq!(format_f64_js(1682.0 / 1e8, 8), "0.00001682");
        assert_eq!(format_f64_js(89679.12, 8), "89679.12");
        assert_eq!(format_f64_js(89679.0, 8), "89679");
        assert_eq!(format_f64_js(0.1, 8), "0.1");
    }
}
