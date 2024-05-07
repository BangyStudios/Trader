mod api;
mod config;
use api::coinspot::CoinSpot;

use tokio;

#[tokio::main]
async fn main() {
    let config = config::Config::init();
    let coinspot = CoinSpot::init(config);
    if let Err(e) = coinspot.get_prices().await {
        eprintln!("Error: {}", e);
    }
}