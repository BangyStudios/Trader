use crate::api::CExClient;
use crate::config::Config;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;

pub fn init_cex_client(config: &crate::config::Config) -> Result<Box<dyn CExClient>> {
    Ok(Box::new(CoinSpot::init(&config)?))
}

pub struct CoinSpot {
    api_key: String, 
    api_secret: String
}

impl CoinSpot {
    pub fn init(config: &Config) -> Result<Self> {
        let api_key = config
            .get_config("coinspot_api_key")
            .ok_or_else(|| anyhow!("Missing coinspot_api_key in config"))?
            .to_string();

        let api_secret = config
            .get_config("coinspot_api_secret")
            .ok_or_else(|| anyhow!("Missing coinspot_api_secret in config"))?
            .to_string();

        Ok(Self { api_key, api_secret })
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
    
        let client = reqwest::Client::new();
        let response = client
            .get("https://www.coinspot.com.au/pubapi/latest")
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


 }