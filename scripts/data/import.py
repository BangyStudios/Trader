#!/usr/bin/env python3
"""Import historical OHLC CSV data into the price_btc table.

Usage:
    python3 scripts/data/import.py <path-to-csv>

Reads DB credentials from config/config.toml (same file used by the Rust app).
CSV is expected to have columns: Start, End, Open, High, Low, Close, Volume, Market Cap
Only the Close price and End date are used, since price_btc has no OHLC/volume columns:
price_buy = price_sell = price_last = Close, timestamp = End date.
"""

import csv
import sys
import tomllib
from datetime import datetime
from pathlib import Path

import mysql.connector

CONFIG_PATH = Path(__file__).resolve().parents[2] / "config" / "config.toml"


def load_config():
    with open(CONFIG_PATH, "rb") as f:
        return tomllib.load(f)


def parse_date(value):
    return datetime.strptime(value.strip(), "%Y-%m-%d")


def main():
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <path-to-csv>", file=sys.stderr)
        sys.exit(1)

    csv_path = Path(sys.argv[1])
    config = load_config()

    conn = mysql.connector.connect(
        user=config["db_user"],
        password=config["db_pass"],
        host=config["db_host"],
        database=config["db_name"],
    )
    cursor = conn.cursor()

    cursor.execute(
        """CREATE TABLE IF NOT EXISTS price_btc (
            id INT AUTO_INCREMENT PRIMARY KEY,
            price_buy DOUBLE NOT NULL,
            price_sell DOUBLE NOT NULL,
            price_last DOUBLE NOT NULL,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
        )"""
    )

    rows = []
    with open(csv_path, newline="") as f:
        reader = csv.DictReader(f)
        for row in reader:
            close_price = float(row["Close"])
            timestamp = parse_date(row["End"])
            rows.append((close_price, close_price, close_price, timestamp))

    cursor.executemany(
        """INSERT INTO price_btc (price_buy, price_sell, price_last, timestamp)
           VALUES (%s, %s, %s, %s)""",
        rows,
    )
    conn.commit()

    print(f"Imported {cursor.rowcount} rows into price_btc.")

    cursor.close()
    conn.close()


if __name__ == "__main__":
    main()
