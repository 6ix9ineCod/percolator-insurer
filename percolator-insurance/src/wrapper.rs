//! InsuredRiskEngine — wraps Percolator's public API with premium collection.
//!
//! Every public method follows the pattern:
//! 1. Collect any accrued premiums owed by the account
//! 2. Execute the Percolator operation
//! 3. Reconcile pool with insurance fund balance

use crate::pool::PremiumPool;
use crate::premium::compute_premium_per_slot;
use crate::risk_index::{
    crowding_multiplier, oi_vault_multiplier, pool_health_multiplier, RiskIndex,
};
use crate::{InsuredError, MULT_SCALE, POS_SCALE};
use percolator::{LiquidationPolicy, RiskEngine, RiskParams, MAX_ACCOUNTS};

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PremiumParams {
    pub base_rate_per_slot: u128,
    pub leverage_exponent_num: u64,
    pub leverage_exponent_den: u64,
    pub min_commitment_slots: u64,
    pub crowding_low_ratio_num: u128,
    pub crowding_low_ratio_den: u128,
    pub crowding_high_ratio_num: u128,
    pub crowding_high_ratio_den: u128,
    pub crowding_cap: u128,
    pub oi_vault_floor_ratio_num: u128,
    pub oi_vault_floor_ratio_den: u128,
    pub oi_vault_cap_ratio_num: u128,
    pub oi_vault_cap_ratio_den: u128,
    pub oi_vault_mult_max: u128,
    pub pool_health_low_num: u128,
    pub pool_health_low_den: u128,
    pub pool_health_high_num: u128,
    pub pool_health_high_den: u128,
    pub pool_health_mult_max: u128,
    pub min_premium_per_slot: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountPremiumState {
    pub last_premium_slot: u64,
    pub commitment_end_slot: u64,
    pub prepaid_premium: u128,
    pub is_active: bool,
}

impl AccountPremiumState {
    pub fn new() -> Self {
        Self {
            last_premium_slot: 0,
            commitment_end_slot: 0,
            prepaid_premium: 0,
            is_active: false,
        }
    }
}

impl Default for AccountPremiumState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct InsuredRiskEngine {
    pub engine: RiskEngine,
    pub pool: PremiumPool,
    pub premium_params: PremiumParams,
    pub account_premiums: [AccountPremiumState; MAX_ACCOUNTS],
}

impl InsuredRiskEngine {
    pub fn new(
        risk_params: RiskParams,
        premium_params: PremiumParams,
        init_slot: u64,
        init_oracle_price: u64,
    ) -> crate::Result<Self> {
        if premium_params.leverage_exponent_den == 0
            || premium_params.crowding_low_ratio_den == 0
            || premium_params.crowding_high_ratio_den == 0
            || premium_params.oi_vault_floor_ratio_den == 0
            || premium_params.oi_vault_cap_ratio_den == 0
            || premium_params.pool_health_low_den == 0
            || premium_params.pool_health_high_den == 0
        {
            return Err(InsuredError::InvalidParams);
        }

        let engine = RiskEngine::new_with_market(risk_params, init_slot, init_oracle_price);

        Ok(Self {
            engine,
            pool: PremiumPool::new(),
            premium_params,
            account_premiums: [AccountPremiumState::new(); MAX_ACCOUNTS],
        })
    }

    pub fn compute_risk_index(&self, account_idx: usize) -> RiskIndex {
        if account_idx >= MAX_ACCOUNTS {
            return RiskIndex::neutral();
        }
        let long_oi = self.engine.oi_eff_long_q;
        let short_oi = self.engine.oi_eff_short_q;
        let vault = self.engine.vault.get();
        let oracle_price = self.engine.last_oracle_price;
        let pp = &self.premium_params;

        let pos = self.engine.accounts[account_idx].position_basis_q;
        let (majority_oi, minority_oi, is_majority) = if long_oi >= short_oi {
            (long_oi, short_oi, pos > 0)
        } else {
            (short_oi, long_oi, pos < 0)
        };

        let crowding = if pos == 0 {
            (MULT_SCALE, MULT_SCALE)
        } else {
            crowding_multiplier(
                majority_oi,
                minority_oi,
                is_majority,
                pp.crowding_low_ratio_num,
                pp.crowding_low_ratio_den,
                pp.crowding_high_ratio_num,
                pp.crowding_high_ratio_den,
                pp.crowding_cap,
            )
        };

        let total_oi_q = long_oi.saturating_add(short_oi);
        let total_oi_notional = if oracle_price > 0 {
            total_oi_q.saturating_mul(oracle_price as u128) / POS_SCALE
        } else {
            0
        };

        let oi_vault = oi_vault_multiplier(
            total_oi_notional,
            vault,
            pp.oi_vault_floor_ratio_num,
            pp.oi_vault_floor_ratio_den,
            pp.oi_vault_cap_ratio_num,
            pp.oi_vault_cap_ratio_den,
            pp.oi_vault_mult_max,
        );

        let pool_health = pool_health_multiplier(
            self.pool.balance,
            total_oi_notional,
            pp.pool_health_low_num,
            pp.pool_health_low_den,
            pp.pool_health_high_num,
            pp.pool_health_high_den,
            pp.pool_health_mult_max,
        );

        RiskIndex {
            crowding,
            oi_vault,
            pool_health,
        }
    }

    fn account_notional(&self, idx: usize) -> u128 {
        let pos = self.engine.accounts[idx].position_basis_q;
        if pos == 0 {
            return 0;
        }
        let abs_pos = pos.unsigned_abs();
        abs_pos.saturating_mul(self.engine.last_oracle_price as u128) / POS_SCALE
    }

    pub fn collect_accrued_premium(
        &mut self,
        idx: u16,
        now_slot: u64,
    ) -> crate::Result<u128> {
        let i = idx as usize;
        if i >= MAX_ACCOUNTS || !self.account_premiums[i].is_active {
            return Ok(0);
        }

        let last = self.account_premiums[i].last_premium_slot;
        if now_slot <= last {
            return Ok(0);
        }
        let slots_elapsed = now_slot - last;

        let notional = self.account_notional(i);
        let capital = self.engine.accounts[i].capital.get();
        let risk_idx = self.compute_risk_index(i);

        let rate = compute_premium_per_slot(
            notional,
            capital,
            self.premium_params.base_rate_per_slot,
            &risk_idx,
            self.premium_params.min_premium_per_slot,
        );

        let premium_owed = rate.saturating_mul(slots_elapsed as u128);
        if premium_owed == 0 {
            self.account_premiums[i].last_premium_slot = now_slot;
            return Ok(0);
        }

        let mut collected = 0u128;
        let mut remaining = premium_owed;

        let prepaid = &mut self.account_premiums[i].prepaid_premium;
        if *prepaid > 0 {
            let from_prepaid = remaining.min(*prepaid);
            remaining -= from_prepaid;
            *prepaid -= from_prepaid;
            collected += from_prepaid;
        }

        if remaining > 0 {
            // The engine caps the fee at the account's available capital and
            // silently drops any excess (charge_fee_to_insurance), returning Ok
            // even when capital is insufficient. Measure the actual insurance-
            // fund delta and record only that — recording the requested amount
            // would over-state the pool's claim on the fund.
            let before = self.engine.insurance_fund.balance.get();
            self.engine
                .charge_account_fee_not_atomic(idx, remaining, now_slot)
                .map_err(InsuredError::Risk)?;
            let after = self.engine.insurance_fund.balance.get();
            let actually_collected = after.saturating_sub(before);
            if actually_collected > 0 {
                self.pool.record_collection(actually_collected)?;
                collected += actually_collected;
            }
        }

        self.account_premiums[i].last_premium_slot = now_slot;
        Ok(collected)
    }

    pub fn reconcile_pool(&mut self) {
        let insurance = self.engine.insurance_fund.balance.get();
        self.pool.reconcile_with_insurance_balance(insurance);
    }

    // ====================================================================
    // Wrapped Percolator operations
    // ====================================================================

    pub fn deposit(
        &mut self,
        idx: u16,
        amount: u128,
        now_slot: u64,
    ) -> crate::Result<()> {
        let i = idx as usize;
        if i < MAX_ACCOUNTS && self.account_premiums[i].is_active {
            let _ = self.collect_accrued_premium(idx, now_slot);
        }

        self.engine
            .deposit_not_atomic(idx, amount, now_slot)
            .map_err(InsuredError::Risk)?;
        Ok(())
    }

    pub fn execute_trade(
        &mut self,
        a: u16,
        b: u16,
        oracle_price: u64,
        now_slot: u64,
        size_q: i128,
        exec_price: u64,
        funding_rate_e9: i128,
        admit_h_min: u64,
        admit_h_max: u64,
        admit_h_max_consumption_threshold_bps_opt: Option<u128>,
    ) -> crate::Result<()> {
        let ai = a as usize;
        let bi = b as usize;

        if ai < MAX_ACCOUNTS && self.account_premiums[ai].is_active {
            let _ = self.collect_accrued_premium(a, now_slot);
        }
        if bi < MAX_ACCOUNTS && self.account_premiums[bi].is_active {
            let _ = self.collect_accrued_premium(b, now_slot);
        }

        let a_was_flat = ai < MAX_ACCOUNTS
            && self.engine.accounts[ai].position_basis_q == 0;
        let b_was_flat = bi < MAX_ACCOUNTS
            && self.engine.accounts[bi].position_basis_q == 0;

        self.engine
            .execute_trade_not_atomic(
                a,
                b,
                oracle_price,
                now_slot,
                size_q,
                exec_price,
                funding_rate_e9,
                admit_h_min,
                admit_h_max,
                admit_h_max_consumption_threshold_bps_opt,
            )
            .map_err(InsuredError::Risk)?;

        if a_was_flat && ai < MAX_ACCOUNTS {
            self.activate_premium(a, now_slot)?;
        }
        if b_was_flat && bi < MAX_ACCOUNTS {
            self.activate_premium(b, now_slot)?;
        }

        if ai < MAX_ACCOUNTS && self.engine.accounts[ai].position_basis_q == 0 {
            self.account_premiums[ai].is_active = false;
        }
        if bi < MAX_ACCOUNTS && self.engine.accounts[bi].position_basis_q == 0 {
            self.account_premiums[bi].is_active = false;
        }

        self.reconcile_pool();
        Ok(())
    }

    fn activate_premium(&mut self, idx: u16, now_slot: u64) -> crate::Result<()> {
        let i = idx as usize;
        if i >= MAX_ACCOUNTS {
            return Ok(());
        }

        let notional = self.account_notional(i);
        let capital = self.engine.accounts[i].capital.get();
        let risk_idx = self.compute_risk_index(i);

        let rate = compute_premium_per_slot(
            notional,
            capital,
            self.premium_params.base_rate_per_slot,
            &risk_idx,
            self.premium_params.min_premium_per_slot,
        );

        let commitment = rate.saturating_mul(self.premium_params.min_commitment_slots as u128);

        let mut charged = 0u128;
        if commitment > 0 {
            // Record only what actually reached the insurance fund: the engine
            // caps the charge at available capital and drops the rest (see
            // collect_accrued_premium). prepaid_premium must reflect the real
            // amount paid, not the requested commitment.
            let before = self.engine.insurance_fund.balance.get();
            self.engine
                .charge_account_fee_not_atomic(idx, commitment, now_slot)
                .map_err(InsuredError::Risk)?;
            let after = self.engine.insurance_fund.balance.get();
            let actually_charged = after.saturating_sub(before);
            if actually_charged > 0 {
                self.pool.record_collection(actually_charged)?;
                charged = actually_charged;
            }
        }

        self.account_premiums[i] = AccountPremiumState {
            last_premium_slot: now_slot,
            commitment_end_slot: now_slot.saturating_add(self.premium_params.min_commitment_slots),
            prepaid_premium: charged,
            is_active: true,
        };

        Ok(())
    }

    pub fn withdraw(
        &mut self,
        idx: u16,
        amount: u128,
        oracle_price: u64,
        now_slot: u64,
        funding_rate_e9: i128,
        admit_h_min: u64,
        admit_h_max: u64,
        admit_h_max_consumption_threshold_bps_opt: Option<u128>,
    ) -> crate::Result<()> {
        let i = idx as usize;
        if i < MAX_ACCOUNTS && self.account_premiums[i].is_active {
            let _ = self.collect_accrued_premium(idx, now_slot);
        }

        self.engine
            .withdraw_not_atomic(
                idx,
                amount,
                oracle_price,
                now_slot,
                funding_rate_e9,
                admit_h_min,
                admit_h_max,
                admit_h_max_consumption_threshold_bps_opt,
            )
            .map_err(InsuredError::Risk)?;
        Ok(())
    }

    pub fn liquidate(
        &mut self,
        idx: u16,
        now_slot: u64,
        oracle_price: u64,
        policy: LiquidationPolicy,
        funding_rate_e9: i128,
        admit_h_min: u64,
        admit_h_max: u64,
        admit_h_max_consumption_threshold_bps_opt: Option<u128>,
    ) -> crate::Result<bool> {
        let i = idx as usize;
        if i < MAX_ACCOUNTS && self.account_premiums[i].is_active {
            let _ = self.collect_accrued_premium(idx, now_slot);
        }

        let result = self
            .engine
            .liquidate_at_oracle_not_atomic(
                idx,
                now_slot,
                oracle_price,
                policy,
                funding_rate_e9,
                admit_h_min,
                admit_h_max,
                admit_h_max_consumption_threshold_bps_opt,
            )
            .map_err(InsuredError::Risk)?;

        if i < MAX_ACCOUNTS && self.engine.accounts[i].position_basis_q == 0 {
            self.account_premiums[i].is_active = false;
        }

        self.reconcile_pool();
        Ok(result)
    }
}
