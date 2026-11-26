use std::fmt;

/// A quantity of one currency, such as a coin ("btc") or a fiat market ("aud"). `currency` is lowercase
#[derive(Clone, Debug, PartialEq)]
pub struct Amount {
    pub currency: String,
    pub amount: f64,
}

impl Amount {
    pub fn init(currency: &str, amount: f64) -> Self {
        Amount { currency: currency.to_ascii_lowercase(), amount }
    }
}

/// Writes `<amount> <CURRENCY>`, honouring a precision such as `{:.2}`
impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let currency = self.currency.to_ascii_uppercase();
        match f.precision() {
            Some(precision) => write!(f, "{:.*} {}", precision, self.amount, currency),
            None => write!(f, "{} {}", self.amount, currency),
        }
    }
}
