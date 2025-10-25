use std::fs;
use toml;

#[derive(Clone)]
pub struct Config {
    data: toml::Value,
}

impl Config {
    pub fn init() -> Self {
        let config_raw = fs::read_to_string("config/config.toml").expect("File cannot be read");
        let data: toml::Value = toml::from_str(&config_raw).unwrap();
        Config { data }
    }

    pub fn get_config(&self, key: &str) -> Option<&str> {
        self.data.get(key).and_then(|value| value.as_str())
    }
}