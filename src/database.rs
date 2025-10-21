pub struct Database {
    connection: sqlite::Connection,
}

impl Database {
    /// Creates a new Database instance and initializes the price_btc table if it doesn't exist
    pub fn init() -> Result<Self, sqlite::Error> {
        // Ensure the data directory exists
        std::fs::create_dir_all("./data");
        
        let connection = sqlite::open("./data/data.db")?;
        
        // Create table if it doesn't exist
        let query = "
            CREATE TABLE IF NOT EXISTS price_btc (
                id INTEGER PRIMARY KEY AUTOINCREMENT,   
                price REAL NOT NULL,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
            )";
        
        connection.execute(query)?;
        
        Ok(Database{connection})
    }

    /// Logs the current BTC price to the database
    pub fn log_price_btc(&self, price: f64) -> Result<(), sqlite::Error> {
        let query = "INSERT INTO price_btc (price) VALUES (:price)";

        let mut statement = self.connection.prepare(query)?;
        
        statement.bind((":price", price))?;
        statement.next();
        
        Ok(())
    }
}

// Example usage with your CoinSpot struct (you'd put this in main.rs or wherever you use it):
/*
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::new(); // Your config initialization
    let coinspot = api::coinspot::CoinSpot::init(config);
    let db = Database::new()?;
    
    // Get BTC price and log it
    if let Some(price_info) = coinspot.get_price_coin("btc").await? {
        if let Some(price) = price_info["last"].as_f64() {
            db.log_btc_price(price)?;
            println!("Logged BTC price: {}", price);
        }
    }
    
    Ok(())
}
*/