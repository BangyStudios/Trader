mod api;
mod config;
use api::coinspot::CoinSpot;

use tokio;

#[tokio::main]
async fn main() {
    let config = config::Config::init();
    let coinspot = CoinSpot::init(config);
    match coinspot.get_price_coin("btc").await {
        Ok(Some(response_json)) => {
            println!("Bid price for BTC: {}", response_json["bid"]);
            println!("Ask price for BTC: {}", response_json["ask"]);
            println!("Last price for BTC: {}", response_json["last"]);
        }
        Ok(None) => println!("Price info not found for BTC"),
        Err(e) => println!("Error: {}", e),
    }
}