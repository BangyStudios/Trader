use crate::algorithm::Algorithm;
use crate::structure::algorithm::{AlgorithmCommand, AlgorithmInput, AlgorithmParams};

use async_trait::async_trait;

/// Dummy algorithm: buys below `price_buy_max`, sells above `price_sell_min`, otherwise idles
pub struct Example {}

#[async_trait]
impl Algorithm for Example {
    async fn run_algorithm(&self, params: AlgorithmParams, input: AlgorithmInput) -> anyhow::Result<AlgorithmCommand> {
        let price_buy_max = 50_000.0; // AUD
        let price_sell_min = 150_000.0; // AUD
        let amount_trade = 100.0; // AUD per transaction

        if input.price_buy < price_buy_max {
            return Ok(AlgorithmCommand { command: 1, amount: Some(amount_trade) });
        }
        if input.price_sell > price_sell_min {
            return Ok(AlgorithmCommand { command: 2, amount: Some(amount_trade) });
        }

        Ok(AlgorithmCommand { command: 0, amount: None })
    }
}
