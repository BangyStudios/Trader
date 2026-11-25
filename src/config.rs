use log;
use serde::de::DeserializeOwned;
use std::fs;
use toml;

#[derive(Clone)]
pub struct Config {
    data: toml::Value,
}

impl Config {
    pub fn init() -> Self {
        let config_raw = fs::read_to_string("./config/config.toml").expect("File cannot be read");
        let data: toml::Value = toml::from_str(&config_raw).unwrap();
        log::info!("Configuration loaded successfully.");
        Config { data }
    }

    /// Returns `key` cast to `Value`, `None` if missing; errors if present with an
    /// uncastable type. TOML integers cast to floats.
    pub fn get_config<Value: DeserializeOwned>(&self, key: &str) -> anyhow::Result<Option<Value>> {
        let value_toml = match self.data.get(key) {
            Some(value_toml) => value_toml.clone(),
            None => return Ok(None),
        };
        let value: Value = match value_toml.try_into() {
            Ok(value) => value,
            Err(e) => {
                let message = e.to_string();
                return Err(anyhow::anyhow!(
                    "Invalid type for '{}' in config: {}",
                    key,
                    message.trim_end()
                ));
            }
        };
        Ok(Some(value))
    }
}
