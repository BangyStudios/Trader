pub mod coingecko;
pub mod coinspot;

use crate::structure::common::Amount;

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug)]
pub struct CExBalanceItem {
    pub amount: Amount,
    amount_fiat: Amount,
    rate: f64,
}

type CExBalance = Vec<CExBalanceItem>;

/// A filled buy order
#[derive(Debug)]
pub struct CExOrderBuy {
    pub amount_in: Amount, // Coin received
    pub amount_out: Amount,   // In the market currency
}

#[async_trait]
pub trait CExClient: Send + Sync {
    async fn pull_prices(&self) -> Result<Value>;
    async fn pull_price_coin(&self, coin: &str) -> Result<Option<Value>>;
    async fn get_sign_body(&self, body: &str) -> String;
    /// Returns the quoted buy rate in the market currency per coin; `amount` is in `coin` or the market currency
    async fn pull_quote_coin_buy(&self, coin: &str, amount: &Amount) -> anyhow::Result<f64>;
    /// Places a buy-now order that the CEx cancels if the rate has risen too far above
    /// `rate`; errors unless the order filled. `amount` is in `coin` or the market currency
    async fn push_order_buy_now(&self, coin: &str, amount: &Amount, rate: f64) -> anyhow::Result<CExOrderBuy>;
    /// Places an order-book buy of `amount` (in the coin) at up to `rate` market currency per coin, waits briefly,
    /// cancels any unfilled rest and returns what filled, with `amount_out` including fees. A zero
    /// `amount_in` means nothing filled
    async fn push_order_buy_market(&self, amount: &Amount, rate: f64) -> anyhow::Result<CExOrderBuy>;
    async fn pull_balance(&self) -> anyhow::Result<CExBalance>;
    fn print_api_key(&self) -> Result<String>;
}
