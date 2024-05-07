mod api;
mod config;
use api::coinspot::CoinSpot;

use tokio;

#[tokio::main]
async fn main() {
    let config = config::Config::init();
    let coinspot = CoinSpot::init(config);
    match coinspot.get_price_coin("btc").await {
        Ok(Some(price_info)) => {
            println!("Bid price for BTC: {}", price_info["bid"]);
            println!("Ask price for BTC: {}", price_info["ask"]);
            println!("Last price for BTC: {}", price_info["last"]);
        }
        Ok(None) => println!("Price info not found for BTC"),
        Err(e) => println!("Error: {}", e),
    }
}