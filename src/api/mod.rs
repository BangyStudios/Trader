pub mod coingecko;
pub mod coinspot;

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct CExBalanceItem {
    currency: String, 
    value_fiat: f64,
    value: f64,
    rate: f64,
}

type CExBalance = Vec<CExBalanceItem>;

#[async_trait]
pub trait CExClient: Send + Sync {
    async fn get_prices(&self) -> Result<Value>;
    async fn get_price_coin(&self, coin: &str) -> Result<Option<Value>>;
    async fn get_sign_body(&self, body: &str) -> String;
    async fn get_quote_coin_buy(
        &self,
        coin: &str,
        amount: &f64,
        amounttype: &str,
    ) -> anyhow::Result<Option<serde_json::Value>>;
    async fn get_balance(&self) -> anyhow::Result<CExBalance>;
    fn print_api_key(&self) -> Result<String>;
}
