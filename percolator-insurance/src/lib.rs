//! Risk-priced premium pool for the Percolator risk engine.
//!
//! Wrapper crate that adds a dynamic, risk-priced *protocol fee* on top of
//! Percolator. Collects per-slot premiums based on leverage, market crowding,
//! system leverage, and pool health, and routes them into Percolator's
//! insurance fund via its public API.
//!
//! NOTE: despite the "insurance" naming, these premiums are a risk-based
//! surcharge feeding the shared solvency buffer — not insurance coverage.
//! There is no policyholder, covered event, or claim payout. See README.md.
//!
//! EDUCATIONAL RESEARCH — NOT PRODUCTION READY, NOT AUDITED.
//!
//! All math is pure integer arithmetic using u256 intermediates.
//! No floating point. no_std compatible.

#![no_std]
#![forbid(unsafe_code)]

pub use percolator::{
    Account, InsuranceFund, RiskEngine, RiskError, RiskParams,
    Result as PercolatorResult, Side, MarketMode, LiquidationPolicy,
    MAX_ACCOUNTS, POS_SCALE, MAX_ORACLE_PRICE, MAX_VAULT_TVL,
    MAX_ACCOUNT_NOTIONAL, MAX_OI_SIDE_Q, FUNDING_DEN,
};

pub mod premium;
pub mod pool;
pub mod risk_index;
pub mod wrapper;

pub use pool::PremiumPool;
pub use premium::{compute_premium_per_slot, isqrt, leverage_multiplier};
pub use risk_index::RiskIndex;
pub use wrapper::{AccountPremiumState, InsuredRiskEngine, PremiumParams};

/// Premium scaling denominator (1e9, matches Percolator's FUNDING_DEN).
pub const PREMIUM_SCALE: u128 = 1_000_000_000;

/// Leverage scaling factor for fixed-point leverage computation.
pub const LEVERAGE_SCALE: u128 = 1_000_000;

/// Multiplier scaling factor. All (num, den) multiplier pairs use this as their denominator when representing 1.0.
pub const MULT_SCALE: u128 = 1_000;

/// Slots per day at 400ms per slot (86_400_000ms / 400ms).
pub const SLOTS_PER_DAY: u64 = 216_000;

/// Error types for the insurance wrapper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsuredError {
    /// Pass-through from Percolator.
    Risk(RiskError),
    /// Account cannot afford the 24h upfront commitment.
    InsufficientForCommitment,
    /// Premium collection failed (account capital exhausted).
    PremiumCollectionFailed,
    /// top_up_insurance_fund rejected (vault TVL cap, time monotonicity).
    PoolTopUpFailed,
    /// Invalid premium parameters at initialization.
    InvalidParams,
}

impl From<RiskError> for InsuredError {
    fn from(e: RiskError) -> Self {
        InsuredError::Risk(e)
    }
}

pub type Result<T> = core::result::Result<T, InsuredError>;
