pub mod coinspot;

use async_trait::async_trait;
use anyhow::Result;
use serde_json::Value;

#[async_trait]
pub trait CExClient: Send + Sync {
    async fn get_prices(&self) -> Result<Value>;
    async fn get_price_coin(&self, coin: &str) -> Result<Option<Value>>;
    fn print_api_key(&self) -> Result<String>;
}