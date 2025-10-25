use crate::api::CExClient;
use crate::config::Config;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use hmac::{Hmac, Mac};
use sha2::Sha512;

#[derive(Serialize)]
struct Payload {
    nonce: String,
}

pub fn init_cex_client(config: &crate::config::Config) -> Result<Box<dyn CExClient>> {
    Ok(Box::new(CoinSpot::init(&config)?))
}

pub struct CoinSpot {
    api_key: String, 
    api_secret: String, 
    client: reqwest::Client, 
    config: Config
}

impl CoinSpot {
    pub fn init(config: &Config) -> Result<Self> {
        let client = reqwest::Client::new();

        let api_key = config
            .get_config("coinspot_api_key")
            .ok_or_else(|| anyhow!("Missing coinspot_api_key in config"))?
            .to_string();

        let api_secret = config
            .get_config("coinspot_api_secret")
            .ok_or_else(|| anyhow!("Missing coinspot_api_secret in config"))?
            .to_string();

        Ok(Self { api_key, api_secret, client, config: config.clone() })
    }
}

#[async_trait]
impl CExClient for CoinSpot {
    fn print_api_key(&self) -> anyhow::Result<String> {
        Ok(format!("API Key: {}, API Secret: {}", self.api_key, self.api_secret))
    }

    async fn get_prices(&self) -> anyhow::Result<serde_json::Value> {
        let mut headers = HeaderMap::new();
        headers.insert("key", HeaderValue::from_str(&self.api_key)?);
        headers.insert("sign", HeaderValue::from_str(&self.api_secret)?);
    
        let response = self.client
            .get("https://www.coinspot.com.au/pubapi/v2/latest")
            .headers(headers)
            .send()
            .await
            .map_err(|e| {
                println!("Request error: {}", e);
                e
            })?;
    
        if response.status().is_success() {
            let response_body = response.text().await.map_err(|e| {
                println!("Response error: {}", e);
                e
            })?;
            let response_json: Value = serde_json::from_str(&response_body).map_err(|e| {
                println!("JSON parsing error: {}", e);
                e
            })?;
            Ok(response_json)
        } else {
            println!("Request failed with status: {}", response.status());
            Err(anyhow::format_err!("Request failed"))
        }
    }

    async fn get_price_coin(&self, coin: &str) -> anyhow::Result<Option<serde_json::Value>> {
        let json_value = self.get_prices().await?;
        let price_info = json_value["prices"][coin].clone();
        Ok(
            if price_info.is_null() { 
                None 
            } else { 
                Some(price_info) 
            }
        )
    }

    async fn get_sign_body(&self, body: &str) -> String {
        let mut mac = Hmac::<Sha512>::new_from_slice(self.api_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(body.as_bytes());
        let sign = hex::encode(mac.finalize().into_bytes());
        
        sign
    }

    async fn get_quote_coin_buy(&self, coin: &str, amount: &f64, amounttype: &str) -> anyhow::Result<Option<serde_json::Value>> {
        let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis()
        .to_string();

        let body = format!(
            r#"{{"nonce":"{}","cointype":"{}","amount":{},"amounttype":"{}"}}"#,
            &nonce,
            &coin,
            &amount, 
            &amounttype
        );

        let sign = self.get_sign_body(&body).await;

        let mut headers = HeaderMap::new();
        headers.insert("key", HeaderValue::from_str(&self.api_key)?);
        headers.insert("sign", HeaderValue::from_str(&sign)?);
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));

        let response = self.client
            .post("https://www.coinspot.com.au/api/v2/quote/buy/now")
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|e| {
                println!("Request error: {}", e);
                e
            })?;
    
        if response.status().is_success() {
            let response_body = response.text().await.map_err(|e| {
                println!("Response error: {}", e);
                e
            })?;
            let response_json: Value = serde_json::from_str(&response_body).map_err(|e| {
                println!("JSON parsing error: {}", e);
                e
            })?;
            Ok(Some(response_json))
        } else {
            println!("Request failed with status: {}, {}", response.status(), response.text().await.unwrap_or_default());
            Err(anyhow::format_err!("Request failed"))
        }
    }
 }