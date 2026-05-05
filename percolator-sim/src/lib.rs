pub mod config;
pub mod data;
pub mod feed;
pub mod signal;
pub mod engine;
pub mod optimizer;
pub mod metrics;

pub use percolator::{MAX_ACCOUNTS, POS_SCALE, MAX_ORACLE_PRICE};
pub use percolator_insurance::{
    InsuredRiskEngine, PremiumParams, PremiumPool, AccountPremiumState,
    PREMIUM_SCALE, LEVERAGE_SCALE, MULT_SCALE, SLOTS_PER_DAY,
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum MarketEvent {
    Trade {
        timestamp_ms: u64,
        price: u64,
        qty: u128,
        is_buy: bool,
    },
    BookUpdate {
        timestamp_ms: u64,
        bids: Vec<(u64, u128)>,
        asks: Vec<(u64, u128)>,
    },
}

pub trait DataSource {
    fn next_event(&mut self) -> Option<MarketEvent>;
}

pub fn price_to_oracle(price_usd: f64) -> u64 {
    (price_usd * POS_SCALE as f64) as u64
}

pub fn qty_to_position(qty: f64) -> u128 {
    (qty * POS_SCALE as f64) as u128
}
