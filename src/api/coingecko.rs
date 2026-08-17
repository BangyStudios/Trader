use anyhow::{Context, Result};
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use serde_json;

use crate::config::Config;

/// Minimal CoinGecko client for pulling historical price data.
pub struct CoinGeckoPriceApi {
    client: Client,
    base_url: String,
    vs_currency: String,
    user_agent: String,
}

impl CoinGeckoPriceApi {
    pub fn new(config: &Config) -> Self {
        let base_url = config
            .get_config("coingecko_base_url")
            .unwrap_or("https://api.coingecko.com/api/v3")
            .trim_end_matches('/')
            .to_string();

        let vs_currency = config
            .get_config("coingecko_vs_currency")
            .unwrap_or("aud")
            .to_ascii_lowercase();

        let user_agent = format!(
            "Trader-IDCA/{}",
            config.get_config("app_version").unwrap_or("dev")
        );

        Self {
            client: Client::new(),
            base_url,
            vs_currency,
            user_agent,
        }
    }

    pub async fn fetch_closing_prices(&self, coin: &str, n_days: u32) -> Result<Vec<f64>> {
        let url = format!("{}/coins/{}/market_chart", self.base_url, coin);

        let n_days = if n_days > 365 {
            365 // CoinGecko limits to 365 days for daily data without API key
        } else {
            n_days
        };
        let query = MarketChartQuery {
            vs_currency: self.vs_currency.as_str(),
            days: n_days,
            interval: "daily",
        };

        let response = self
            .client
            .get(url)
            .header(header::USER_AGENT, &self.user_agent)
            .query(&query)
            .send()
            .await
            .with_context(|| format!("Failed to reach CoinGecko for {coin}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::format_err!(
                "CoinGecko request failed with status {}: {}",
                status,
                body
            ));
        }

        let body = response
            .text()
            .await
            .context("Failed to read CoinGecko response body")?;

        let payload: MarketChartResponse = serde_json::from_str(&body)
            .context("Unable to parse CoinGecko response body")?;

        let closes: Vec<f64> = payload.prices.into_iter().map(|entry| entry[1]).collect();

        if closes.is_empty() {
            Err(anyhow::format_err!("CoinGecko returned no price data"))
        } else {
            Ok(closes)
        }
    }

    pub async fn fetch_average_price(&self, coin: &str, n_days: u32) -> Result<f64> {
        let prices = self.fetch_closing_prices(coin, n_days).await?;
        let sum: f64 = prices.iter().sum();
        Ok(sum / (prices.len() as f64))
    }
}

#[derive(Serialize)]
struct MarketChartQuery<'a> {
    vs_currency: &'a str,
    days: u32,
    interval: &'a str,
}

#[derive(Deserialize)]
struct MarketChartResponse {
    prices: Vec<[f64; 2]>,
}
