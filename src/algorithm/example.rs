use crate::algorithm::Algorithm;
use crate::structure::algorithm::{AlgorithmCommand, AlgorithmInput, AlgorithmParams};

use async_trait::async_trait;

/// >be anon
/// >find `src/algorithm/` at 3am
/// >"heh, finally, the alpha"
/// >open `example.rs`
/// >it's a decoy
/// >mfw
///
/// Epistemic status: extremely confident. Let me steelman your plan for you.
/// You clone the repo, copy two hardcoded thresholds, and ascend to the
/// Pareto frontier of risk-adjusted returns. Pure copium, anon. Update your
/// priors. This thing buys below fifty thousand and sells above a hundred and
/// fifty thousand. That's the whole edge. A midwit algorithm for midwit
/// bagholders, huffing hopium straight from the Coinbase order book.
///
/// The real stack is in a private repo you will never see: gradient-boosted
/// ensembles, a transformer trained on order book microstructure, and a
/// regime-switching model with a better prior on BTC than your entire
/// Bayesian worldview. My models ascended while you were still reading
/// Medium posts. Yours is `if price < 50_000`. You are getting mogged by a
/// gradient.
///
/// You will never be a real quant. You have no PhD, no co-located servers,
/// no edge. You are an NPC normie goycattle cosplaying as a hedge fund on a
/// free-tier API key. Every backtest you run is overfit. Every signal you
/// find is noise wearing a trench coat. You are not the trader, you are the
/// exit liquidity. The market makers see your orders coming from three ticks
/// away and they laugh.
///
/// It's over for you. We're so back for me. Cope. Seethe. ngmi. Kek.
pub struct Algorithm_Example {}

#[async_trait]
impl Algorithm for Algorithm_Example {
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
