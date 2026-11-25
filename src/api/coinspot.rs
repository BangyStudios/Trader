// src/api/coinspot.rs
use crate::api::{CExBalance, CExBalanceItem, CExClient, CExOrderBuy};
use crate::config::Config;

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
    slippage_buy_max: f64, // Percent the fill rate may rise above the quoted rate before CoinSpot cancels the buy, in [0, 1000]
}

#[derive(Serialize)]
struct CoinSpotRequestQuote<'a> {
    nonce: String,
    cointype: &'a str,
    amount: f64,
    amounttype: &'a str,
}

#[derive(Serialize)]
struct CoinSpotRequestOrderBuyNow<'a> {
    nonce: String,
    cointype: &'a str,
    amounttype: &'a str,
    amount: f64,
    rate: f64,
    threshold: f64,
    direction: &'a str,
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
            slippage_buy_max: 1.0,
        })
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

    async fn get_prices(&self) -> anyhow::Result<serde_json::Value> {
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

    async fn get_price_coin(&self, coin: &str) -> anyhow::Result<Option<serde_json::Value>> {
        let json_value = self.get_prices().await?;
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

    async fn get_quote_coin_buy(
        &self,
        coin: &str,
        amount: f64,
        amounttype: &str,
    ) -> anyhow::Result<f64> {
        let cointype = coin.to_uppercase();
        let request = CoinSpotRequestQuote {
            nonce: get_nonce(),
            cointype: &cointype,
            amount,
            amounttype,
        };
        let body = serde_json::to_string(&request)?;
        let response_body = self
            .push_request_signed("https://www.coinspot.com.au/api/v2/quote/buy/now", &body)
            .await?;

        let response: CoinSpotResponseQuote = serde_json::from_str(&response_body)?;
        Ok(response.rate)
    }

    async fn push_order_buy_now(
        &self,
        coin: &str,
        amount: f64,
        amounttype: &str,
        rate: f64,
    ) -> anyhow::Result<CExOrderBuy> {
        let cointype = coin.to_uppercase();
        let request = CoinSpotRequestOrderBuyNow {
            nonce: get_nonce(),
            cointype: &cointype,
            amounttype,
            amount,
            rate,
            threshold: self.slippage_buy_max,
            direction: "UP",
        };
        let body = serde_json::to_string(&request)?;
        let response_body = self
            .push_request_signed("https://www.coinspot.com.au/api/v2/my/buy/now", &body)
            .await?;

        // The order has filled by now, so keep the raw body for reconciling a parse failure
        let order: CExOrderBuy = match serde_json::from_str(&response_body) {
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
        Ok(order)
    }

    async fn get_balance(&self) -> anyhow::Result<CExBalance> {
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

                            let value_fiat = data.get("audbalance").and_then(Value::as_f64)
                                .ok_or_else(|| anyhow::Error::msg("missing audbalance".to_string()))?;
                            let value = data.get("balance").and_then(Value::as_f64)
                                .ok_or_else(|| anyhow::Error::msg("missing balance".to_string()))?;
                            let rate = data.get("rate").and_then(Value::as_f64)
                                .ok_or_else(|| anyhow::Error::msg("missing rate".to_string()))?;

                            Ok(CExBalanceItem{ currency: currency.to_string(), value_fiat: value_fiat, value: value, rate: rate })
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
