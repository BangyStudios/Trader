pub mod coingecko;
pub mod coinspot;

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct CExBalanceItem {
    pub currency: String, 
    value_fiat: f64,
    pub value: f64,
    rate: f64,
}

type CExBalance = Vec<CExBalanceItem>;

/// A filled buy order
#[derive(Debug, Deserialize)]
pub struct CExOrderBuy {
    pub coin: String,
    pub market: String,
    pub amount: f64, // Coin received
    pub total: f64,  // Paid, in the market currency (AUD)
}

#[async_trait]
pub trait CExClient: Send + Sync {
    async fn pull_prices(&self) -> Result<Value>;
    async fn pull_price_coin(&self, coin: &str) -> Result<Option<Value>>;
    async fn get_sign_body(&self, body: &str) -> String;
    /// Returns the quoted buy rate in AUD per coin
    async fn pull_quote_coin_buy(&self, coin: &str, amount: f64, amounttype: &str)
        -> anyhow::Result<f64>;
    /// Places a buy-now order that the CEx cancels if the rate has risen too far above
    /// `rate`; errors unless the order filled
    async fn push_order_buy_now(
        &self,
        coin: &str,
        amount: f64,
        amounttype: &str,
        rate: f64,
    ) -> anyhow::Result<CExOrderBuy>;
    /// Places an order-book buy of `amount_coin` at up to `rate` AUD per coin, waits briefly, cancels any
    /// unfilled rest and returns what filled, with `total` including fees. A zero `amount` means nothing filled
    async fn push_order_buy_market(&self, coin: &str, amount_coin: f64, rate: f64) -> anyhow::Result<CExOrderBuy>;
    async fn pull_balance(&self) -> anyhow::Result<CExBalance>;
    fn print_api_key(&self) -> Result<String>;
}
