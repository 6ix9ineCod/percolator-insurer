pub mod accounts;
pub mod clock;

use accounts::AccountManager;
use clock::SlotClock;
use crate::metrics::{MetricsCollector, Snapshot};
use crate::signal::FlowSignal;
use crate::{MarketEvent, POS_SCALE};
use percolator::{
    LiquidationPolicy, RiskParams, U128, MAX_ACCOUNTS,
};
use percolator_insurance::{
    InsuredRiskEngine, PremiumParams, InsuredError,
};

pub struct SimEngine {
    pub engine: InsuredRiskEngine,
    pub clock: SlotClock,
    pub accounts: AccountManager,
    pub metrics: MetricsCollector,
    pub signal: FlowSignal,
    last_oracle_price: u64,
    last_snapshot_slot: u64,
    last_accrual_slot: u64,
    position_age: [u64; MAX_ACCOUNTS],
    max_position_slots: u64,
}

impl SimEngine {
    pub fn new(premium_params: PremiumParams, slot_duration_ms: u64, sample_interval: u64) -> Self {
        let risk_params = RiskParams {
            maintenance_margin_bps: 500,
            initial_margin_bps: 1000,
            trading_fee_bps: 10,
            max_accounts: 64,
            liquidation_fee_bps: 100,
            liquidation_fee_cap: U128::new(1_000_000_000),
            min_liquidation_abs: U128::new(0),
            min_nonzero_mm_req: 10,
            min_nonzero_im_req: 11,
            h_min: 0,
            h_max: 100,
            resolve_price_deviation_bps: 1000,
            max_accrual_dt_slots: 100,
            max_abs_funding_e9_per_slot: 10_000,
            min_funding_lifetime_slots: 10_000_000,
            max_active_positions_per_side: MAX_ACCOUNTS as u64,
            max_price_move_bps_per_slot: 3,
        };

        let engine = InsuredRiskEngine::new(risk_params, premium_params, 0, 1).unwrap();

        Self {
            engine,
            clock: SlotClock::new(0, slot_duration_ms),
            accounts: AccountManager::new(60, 4),
            metrics: MetricsCollector::new(sample_interval),
            signal: FlowSignal::new(),
            last_oracle_price: 1,
            last_snapshot_slot: 0,
            last_accrual_slot: 0,
            position_age: [0u64; MAX_ACCOUNTS],
            max_position_slots: 54_000, // ~6 hours at 400ms slots
        }
    }

    pub fn initialize(&mut self, oracle_price: u64, vault_seed: u128, fund_seed: u128) {
        self.last_oracle_price = oracle_price;
        let lps = self.accounts.lp_accounts();
        let per_lp = vault_seed / lps.len() as u128;
        for &lp in lps {
            let _ = self.engine.deposit(lp, per_lp, 0);
        }
        if fund_seed > 0 {
            let _ = self.engine.engine.top_up_insurance_fund(fund_seed, 0);
        }
    }

    pub fn process_event(&mut self, event: &MarketEvent) -> Result<(), SimError> {
        match event {
            MarketEvent::Trade { timestamp_ms, price, qty, is_buy } => {
                let slots_advanced = self.clock.advance_to(*timestamp_ms);
                let now_slot = self.clock.current_slot();

                if *price > 0 {
                    self.last_oracle_price = *price;
                }

                self.signal.volume.record_trade(*timestamp_ms, (*qty).min(u64::MAX as u128) as u64, *is_buy);
                self.signal.aggression.record(*timestamp_ms, (*qty).min(u64::MAX as u128) as u64, *is_buy);

                let notional = (*qty as u128).saturating_mul(self.last_oracle_price as u128) / POS_SCALE;
                self.metrics.record_trade_notional(notional);

                if let Some(acct_idx) = self.accounts.allocate_trade_account() {
                    let leverage = 10u128;
                    let capital = notional / leverage;
                    if capital > 0 {
                        let _ = self.engine.deposit(acct_idx, capital.max(100), now_slot);

                        let lp = self.accounts.lp_accounts()[0];
                        let size_q = (*qty).min(i128::MAX as u128) as i128;
                        if size_q > 0 {
                            let (a, b) = if *is_buy { (acct_idx, lp) } else { (lp, acct_idx) };
                            match self.engine.execute_trade(
                                a, b, self.last_oracle_price, now_slot,
                                size_q, self.last_oracle_price, 0, 0, 100, None,
                            ) {
                                Ok(()) => {
                                    self.accounts.mark_positioned(acct_idx);
                                    self.position_age[acct_idx as usize] = now_slot;
                                }
                                Err(_) => {
                                    self.accounts.release_trade_account(acct_idx);
                                }
                            }
                        } else {
                            self.accounts.release_trade_account(acct_idx);
                        }
                    } else {
                        self.accounts.release_trade_account(acct_idx);
                    }
                }

                if slots_advanced > 0 {
                    self.run_slot_maintenance(now_slot, *timestamp_ms);
                }
            }
            MarketEvent::BookUpdate { timestamp_ms, bids, asks } => {
                self.clock.advance_to(*timestamp_ms);
                let bid_levels: Vec<(u64, u64)> = bids.iter()
                    .map(|&(p, q)| (p, q.min(u64::MAX as u128) as u64))
                    .collect();
                let ask_levels: Vec<(u64, u64)> = asks.iter()
                    .map(|&(p, q)| (p, q.min(u64::MAX as u128) as u64))
                    .collect();
                self.signal.depth.update(&bid_levels, &ask_levels);
            }
        }
        Ok(())
    }

    fn run_slot_maintenance(&mut self, now_slot: u64, timestamp_ms: u64) {
        let do_accrual = now_slot >= self.last_accrual_slot + 100;
        if do_accrual {
            self.last_accrual_slot = now_slot;
        }

        // Snapshot positioned indices into a stack buffer to avoid borrow conflicts
        let mut active = [0u16; MAX_ACCOUNTS];
        let mut active_count = 0usize;
        for i in 0..60u16 {
            if self.accounts.is_positioned(i) {
                active[active_count] = i;
                active_count += 1;
            }
        }

        let lps = self.accounts.lp_accounts();
        let lp_count = lps.len();
        let mut lp_arr = [0u16; 4];
        lp_arr[..lp_count].copy_from_slice(lps);

        for ai in 0..active_count {
            let idx = active[ai];
            let i = idx as usize;

            if do_accrual {
                let _ = self.engine.collect_accrued_premium(idx, now_slot);
            }

            if self.position_age[i] == 0 {
                self.position_age[i] = now_slot;
            }
            let age = now_slot.saturating_sub(self.position_age[i]);
            if age >= self.max_position_slots {
                let pos = self.engine.engine.try_effective_pos_q(i).unwrap_or(0);
                if pos != 0 {
                    let abs_pos = pos.unsigned_abs().min(i128::MAX as u128) as i128;
                    let lp = lp_arr[i % lp_count];
                    let (a, b) = if pos > 0 { (lp, idx) } else { (idx, lp) };
                    if self.engine.execute_trade(
                        a, b, self.last_oracle_price, now_slot,
                        abs_pos, self.last_oracle_price, 0, 0, 100, None,
                    ).is_err() {
                        continue;
                    }
                }
                self.position_age[i] = 0;
                self.accounts.mark_flat(idx);
                self.accounts.release_trade_account(idx);
                continue;
            }

            match self.engine.liquidate(
                idx, now_slot, self.last_oracle_price,
                LiquidationPolicy::FullClose, 0, 0, 100, None,
            ) {
                Ok(true) => {
                    let capital = self.engine.engine.accounts[i].capital.get();
                    self.metrics.record_liquidation(now_slot, capital);
                    self.position_age[i] = 0;
                    self.accounts.mark_flat(idx);
                    self.accounts.release_trade_account(idx);
                }
                _ => {}
            }
        }

        self.maybe_snapshot(now_slot, timestamp_ms);
    }

    fn maybe_snapshot(&mut self, slot: u64, timestamp_ms: u64) {
        if slot < self.last_snapshot_slot + self.metrics.sample_interval {
            return;
        }
        self.last_snapshot_slot = slot;

        let (h_num, h_den) = self.engine.engine.haircut_ratio();
        let active = self.accounts.positioned_accounts().len() as u32;
        let tox = self.signal.toxicity(timestamp_ms);

        self.metrics.record(Snapshot {
            slot,
            timestamp_ms,
            insurance_fund_balance: self.engine.engine.insurance_fund.balance.get(),
            pool_balance: self.engine.pool.balance,
            pool_total_collected: self.engine.pool.total_collected,
            pool_total_paid_out: self.engine.pool.total_paid_out,
            haircut_num: h_num,
            haircut_den: h_den,
            vault_balance: self.engine.engine.vault.get(),
            total_oi_long: self.engine.engine.oi_eff_long_q,
            total_oi_short: self.engine.engine.oi_eff_short_q,
            active_accounts: active,
            flow_toxicity: tox,
        });
    }

    pub fn fund_balance(&self) -> u128 {
        self.engine.engine.insurance_fund.balance.get()
    }

    pub fn conservation_ok(&self) -> bool {
        self.engine.engine.check_conservation()
    }
}

#[derive(Debug)]
pub enum SimError {
    Engine(InsuredError),
    Data(String),
}

impl From<InsuredError> for SimError {
    fn from(e: InsuredError) -> Self {
        SimError::Engine(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MarketEvent, POS_SCALE};
    use percolator_insurance::PremiumParams;

    fn test_premium_params() -> PremiumParams {
        PremiumParams {
            base_rate_per_slot: 100,
            leverage_exponent_num: 3,
            leverage_exponent_den: 2,
            min_commitment_slots: 216_000,
            crowding_low_ratio_num: 1500,
            crowding_low_ratio_den: 1000,
            crowding_high_ratio_num: 5000,
            crowding_high_ratio_den: 1000,
            crowding_cap: 4000,
            oi_vault_floor_ratio_num: 1,
            oi_vault_floor_ratio_den: 1,
            oi_vault_cap_ratio_num: 5,
            oi_vault_cap_ratio_den: 1,
            oi_vault_mult_max: 3000,
            pool_health_low_num: 1,
            pool_health_low_den: 100,
            pool_health_high_num: 5,
            pool_health_high_den: 100,
            pool_health_mult_max: 5000,
            min_premium_per_slot: 1,
            // Disabled: preserve pre-existing sim economics (opt in to price these later)
            volatility_mult_num: 1_000,
            volatility_mult_den: 1_000,
            leverage_tail_threshold_bps: 10_000,
            leverage_tail_steepness: 0,
            collection_maint_buffer_bps: 0,
            max_oracle_deviation_bps: 0,
            max_oracle_staleness_slots: 0,
            require_authorization: false,
        }
    }

    #[test]
    fn sim_engine_initializes() {
        let se = SimEngine::new(test_premium_params(), 400, 100);
        assert_eq!(se.clock.current_slot(), 0);
        assert_eq!(se.accounts.free_count(), 60);
    }

    #[test]
    fn process_trade_event_opens_position() {
        let mut se = SimEngine::new(test_premium_params(), 400, 100);
        let price: u64 = 50_000 * POS_SCALE as u64;
        se.initialize(price, 1_000_000_000, 0);
        let event = MarketEvent::Trade {
            timestamp_ms: 400,
            price,
            qty: 1 * POS_SCALE as u128,
            is_buy: true,
        };
        let result = se.process_event(&event);
        assert!(result.is_ok());
    }
}
