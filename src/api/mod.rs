pub mod coinspot;

use async_trait::async_trait;
use anyhow::Result;
use serde_json::Value;

#[async_trait]
pub trait CExClient: Send + Sync {
    async fn get_prices(&self) -> Result<Value>;
    async fn get_price_coin(&self, coin: &str) -> Result<Option<Value>>;
    async fn get_sign_body(&self, body: &str) -> String;
    async fn get_quote_coin_buy(&self, coin: &str, amount: &f64) -> anyhow::Result<Option<serde_json::Value>>;
    fn print_api_key(&self) -> Result<String>;
}