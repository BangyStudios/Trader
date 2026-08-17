use async_trait::async_trait;
use std::collections::HashMap;

use crate::{algorithm::Algorithm, structure::algorithm::{AlgorithmCommand, AlgorithmInput}};

pub struct Algorithm_Buy_v1_0 {}

#[async_trait]
impl Algorithm for Algorithm_Buy_v1_0 {
    async fn run_algorithm(&self, input: AlgorithmInput) -> anyhow::Result<AlgorithmCommand> {
        // Implement your buy algorithm logic here
        // For example, analyze market data and decide whether to buy

        let mut result: AlgorithmCommand = AlgorithmCommand {
            command: "idle".to_string(), 
            amount: Some(10.00)
        };
        Ok(result)
    }
}
