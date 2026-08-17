pub mod v1_0;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

use crate::structure::algorithm::{AlgorithmCommand, AlgorithmInput};

#[async_trait]
pub trait Algorithm {
    async fn run_algorithm(&self, input: AlgorithmInput) -> anyhow::Result<AlgorithmCommand>;
}
