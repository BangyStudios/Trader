mod algorithm;
mod api;
mod config;
mod daemon;
mod database;

use tokio;
use api::CExClient;

#[tokio::main]
async fn main() {
    let daemon = match daemon::Daemon::new("coinspot") {
        Ok(daemon) => daemon, 
        Err(e) => panic!("Problem initializing the daemon: {e:?}")
    };
    daemon.run().await;
}