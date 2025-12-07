// src/main.rs
mod algorithm;
mod api;
mod config;
mod daemon;
mod database;

use env_logger;
use tokio;

#[tokio::main]
async fn main() {
    env_logger::init();

    log::info!("Starting Trader Daemon...");
    let daemon = match daemon::Daemon::new("coinspot") {
        Ok(daemon) => daemon,
        Err(e) => {
            log::error!("Problem initializing the daemon: {:?}", e);
            std::process::exit(1);
        }
    };
    daemon.run().await;
}