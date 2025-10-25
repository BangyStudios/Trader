pub mod v1_0;

use async_trait::async_trait;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;

#[async_trait]
pub trait Algorithm {
    async fn run_algorithm(&self) -> anyhow::Result<HashMap<String, Value>>;
}