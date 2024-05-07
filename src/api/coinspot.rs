use crate::config::Config;

use reqwest;
use reqwest::header::{ HeaderMap, HeaderValue };
use serde_json::Value;

pub struct CoinSpot {
    config: Config, 
    api_key: String, 
    api_secret: String
}

impl CoinSpot {
    pub fn init(config: Config) -> Self {
        let api_key = config.get("coinspot_api_key").unwrap_or("Fuck!").to_string();
        let api_secret = config.get("coinspot_api_secret").unwrap_or("Fuck!").to_string();
        CoinSpot { config, api_key, api_secret }
    }

    pub fn print(&self) -> String {
        format!("API Key: {}, API Secret: {}", self.api_key, self.api_secret)
    }

    pub async fn get_prices(&self) -> Result<Value, Box<dyn std::error::Error>> {
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
            Err("Request failed".into())
        }
    }

    pub async fn get_price_coin(&self, coin: &str) -> Result<Option<Value>, Box<dyn std::error::Error>> {
        let json_value = self.get_prices().await?;
        let price_info = json_value["prices"][coin].clone();
        Ok(if price_info.is_null() { None } else { Some(price_info) })
    }
 }