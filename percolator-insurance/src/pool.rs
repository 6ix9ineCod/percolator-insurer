//! Premium pool accounting.
//!
//! Tracks premium funds as a claim on Percolator's insurance fund.
//! The pool does not hold funds separately — it records how much of
//! Percolator's insurance fund balance originated from premiums.

use crate::InsuredError;

/// Accounting record of premium contributions to Percolator's insurance fund.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PremiumPool {
    /// Accounting claim on Percolator's insurance fund balance.
    pub balance: u128,
    /// Lifetime premiums collected (monotonically increasing).
    pub total_collected: u128,
    /// Lifetime consumed by deficit coverage (monotonically increasing).
    pub total_paid_out: u128,
    /// Slot of last deficit reconciliation.
    pub last_deficit_check_slot: u64,
}

impl PremiumPool {
    /// Create a zeroed pool.
    pub fn new() -> Self {
        Self {
            balance: 0,
            total_collected: 0,
            total_paid_out: 0,
            last_deficit_check_slot: 0,
        }
    }

    /// Record an incoming premium collection.
    ///
    /// Adds `amount` to both `balance` and `total_collected`.
    /// Returns `InsuredError::InvalidParams` on overflow.
    pub fn record_collection(&mut self, amount: u128) -> crate::Result<()> {
        let new_balance = self
            .balance
            .checked_add(amount)
            .ok_or(InsuredError::InvalidParams)?;
        let new_collected = self
            .total_collected
            .checked_add(amount)
            .ok_or(InsuredError::InvalidParams)?;
        self.balance = new_balance;
        self.total_collected = new_collected;
        Ok(())
    }

    /// Record consumption of premium funds against a deficit.
    ///
    /// Capped at the current balance — cannot consume more than is available.
    /// `total_paid_out` is incremented by the actual consumed amount.
    pub fn record_consumption(&mut self, amount: u128) {
        let actual = amount.min(self.balance);
        self.balance -= actual;
        self.total_paid_out = self.total_paid_out.saturating_add(actual);
    }

    /// Reconcile the pool's accounting balance against the real insurance fund balance.
    ///
    /// If the insurance fund balance is lower than the pool's recorded balance,
    /// the difference has already been consumed (deficit coverage). This method
    /// calls `record_consumption` for that shortfall and returns the amount consumed.
    /// Returns 0 if there is no deficit.
    pub fn reconcile_with_insurance_balance(&mut self, insurance_balance: u128) -> u128 {
        if self.balance <= insurance_balance {
            return 0;
        }
        let consumed = self.balance - insurance_balance;
        self.record_consumption(consumed);
        consumed
    }

    /// Verify the pool's internal accounting invariants.
    ///
    /// Returns `true` when:
    /// - `balance + total_paid_out == total_collected` (no overflow)
    /// - `total_paid_out <= total_collected`
    pub fn check_invariants(&self) -> bool {
        match self.balance.checked_add(self.total_paid_out) {
            Some(sum) => sum == self.total_collected && self.total_paid_out <= self.total_collected,
            None => false,
        }
    }
}

impl Default for PremiumPool {
    fn default() -> Self {
        Self::new()
    }
}
