use crate::config::Config;
use reqwest;
use reqwest::header::{ HeaderMap, HeaderValue };

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

    pub async fn get_prices(&self) -> Result<(), Box<dyn std::error::Error>> {

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
            let body = response.text().await.map_err(|e| {
                println!("Response error: {}", e);
                e
            })?;
            println!("Response: {}\n", body);
        } else {
            println!("Request failed with status: {}\n", response.status());
        }
        Ok(())
    }
 }