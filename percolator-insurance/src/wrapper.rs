//! InsuredRiskEngine — wraps Percolator's public API with premium collection.
//!
//! Every public method follows the pattern:
//! 1. Collect any accrued premiums owed by the account
//! 2. Execute the Percolator operation
//! 3. Reconcile pool with insurance fund balance
//!
//! ─────────────────────────────────────────────────────────────────────────
//! WS2 Task 1 — recurring-fee API: WHY WE DO NOT USE
//! `sync_account_fee_to_slot_not_atomic`, AND WHY THAT IS DOUBLE-CHARGE-SAFE
//! ─────────────────────────────────────────────────────────────────────────
//!
//! The engine exposes two fee entrypoints, and the README says recurring
//! time-based fees "MUST" be synced via `sync_account_fee_to_slot_not_atomic`.
//! We deliberately keep using the one-shot `charge_account_fee_not_atomic`
//! with the crate's own `last_premium_slot` bookkeeping. Justification:
//!
//! 1. BOTH paths bottom out in the same primitive. `charge_account_fee_not_atomic`
//!    calls `charge_fee_to_insurance(idx, fee_abs)`. `sync_account_fee_to_slot_not_atomic`
//!    computes `fee_abs = fee_rate_per_slot * dt` (capped at MAX_PROTOCOL_FEE_ABS)
//!    and then calls the SAME `charge_fee_to_insurance(idx, fee_abs)`. So the
//!    actual money movement, the capital-cap behaviour, and the insurance-fund
//!    delta are identical. There is no economic difference in WHAT reaches the
//!    fund — only in WHO owns the time anchor.
//!
//! 2. The recurring API charges a FLAT `rate * dt`. Our premium is NOT flat:
//!    `compute_premium_per_slot` re-prices every accrual from the live risk
//!    index (crowding, oi/vault, pool-health, volatility, leverage-tail). A
//!    single `fee_rate_per_slot` cannot express a rate that changes within the
//!    interval. Migrating would force us to either (a) freeze the rate across
//!    the interval — losing the risk responsiveness that is the entire point of
//!    this crate — or (b) call sync once per slot, which is gas-prohibitive and
//!    still re-prices identically to what we already do. The parallel
//!    bookkeeping is the correct shape for a dynamic rate.
//!
//! 3. DOUBLE-CHARGE SAFETY. The only way the recurring API and our path could
//!    double-charge is if BOTH advanced an overlapping interval against the same
//!    account. They cannot, because they use DISJOINT anchors. The engine's
//!    recurring path owns `Account::last_fee_slot` and charges over
//!    `[last_fee_slot, anchor]`; this crate owns
//!    `AccountPremiumState::last_premium_slot` and charges over
//!    `[last_premium_slot, now_slot]`. These are two SEPARATE checkpoints. This
//!    crate NEVER calls `sync_account_fee_to_slot_not_atomic` (grep-verifiable:
//!    the symbol does not appear anywhere in this crate), so `last_fee_slot` is
//!    only ever advanced by the engine's internal one-shot path — which charges
//!    instantaneously and does not back-charge an interval. Therefore no
//!    interval is ever charged twice: our path is the sole driver of
//!    `last_premium_slot`, and it advances it monotonically on every collection
//!    (including the zero-premium early-returns), so an interval `[a, b]` is
//!    consumed exactly once. The integrator MUST NOT also enable engine
//!    recurring fees on these same accounts; doing so would be a configuration
//!    error (two independent rate schedules), not a flaw in this path.
//!
//! DECISION: keep the one-shot + `last_premium_slot` design. Migration is
//! strictly higher risk (loses dynamic pricing, identical fund effect) with no
//! offsetting benefit.
//!
//! ─────────────────────────────────────────────────────────────────────────
//! WS2 Task 2 — discarded-result (collection failure) policy
//! ─────────────────────────────────────────────────────────────────────────
//!
//! Wrapped ops collect accrued premium BEFORE the underlying engine call. The
//! prior code did `let _ = self.collect_accrued_premium(..)`, swallowing any
//! error. New policy: PROPAGATE. A collection that returns `Err` means the
//! engine refused the fee charge (e.g. wrong market mode, time-monotonicity
//! violation, post-condition failure) — a real fault, not "insufficient
//! capital" (which the engine reports as `Ok` with a capped delta, already
//! handled by the actual-delta recording). We therefore surface the error and
//! ABORT the wrapped op: it is unsafe to mutate position/capital on top of an
//! engine that just rejected a fee touch for the same account. The
//! capital-insufficient case is NOT a failure here and continues to return Ok,
//! so this policy does not regress
//! `test_pool_records_only_actual_collection_when_capital_insufficient`.

use crate::pool::PremiumPool;
use crate::premium::{compute_premium_per_slot, leverage_tail_surcharge};
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

    // ── Task 2: realized-volatility multiplier (governance param) ────────────
    /// Governance-set realized-volatility multiplier `(num, den)`, scaled by
    /// `MULT_SCALE`. Default neutral `(MULT_SCALE, MULT_SCALE)` = 1.0x.
    ///
    /// CALIBRATION REQUIRED: a production deployment MUST feed an
    /// oracle-realized-volatility multiplier here (e.g. EWMA/realized-vol of the
    /// mark over a trailing window, normalised so 1.0x = baseline vol). The
    /// covered loss is gap risk, which scales with volatility; leaving this
    /// neutral under-prices premiums in turbulent regimes. There is no
    /// calibrated on-chain vol source in this crate yet, so `compute_risk_index`
    /// passes this param straight through unchanged.
    pub volatility_mult_num: u128,
    pub volatility_mult_den: u128,

    // ── Task 3: leverage tail-surcharge (governance params) ──────────────────
    /// Fraction of the maintenance-margin limit (`1/maintenance_margin_bps`)
    /// at which the tail surcharge begins, in basis points of that limit.
    /// e.g. `8000` = surcharge starts once leverage reaches 80% of the maximum
    /// permissible leverage `L_max = 10_000 / maintenance_margin_bps`.
    ///
    /// CALIBRATION REQUIRED: the exact onset/steepness of this curve must be fit
    /// to an empirical loss distribution near the maintenance boundary, where
    /// the liquidation buffer collapses and `leverage^1.5` under-prices the tail.
    pub leverage_tail_threshold_bps: u128,
    /// Steepness of the tail surcharge at the maintenance boundary, scaled by
    /// `MULT_SCALE`. The surcharge multiplier ramps linearly from 1.0x at the
    /// threshold to `1.0x + (steepness/MULT_SCALE)` as leverage reaches
    /// `L_max`. e.g. `3000` (3.0 in MULT_SCALE) → up to a 4.0x surcharge at the
    /// boundary, multiplied on top of the existing `leverage^1.5` factor.
    ///
    /// CALIBRATION REQUIRED: fit to the loss distribution (see above).
    pub leverage_tail_steepness: u128,

    // ── WS2 Task 3: counter-cyclical collection cap (governance param) ───────
    /// Safety buffer, in basis points of the account's current notional, that
    /// a single premium collection (or commitment charge) must leave ABOVE the
    /// account's maintenance-margin requirement.
    ///
    /// Motivation (actuarial review): under stress the `pool_health` multiplier
    /// spikes premiums, and an uncapped collection can draw a near-maintenance
    /// account down to its full remaining capital. That shrinks the very
    /// liquidation buffer the fund relies on, manufacturing the socialized
    /// deficits the pool must then cover — a pro-cyclical death spiral. This
    /// guard caps each collection so post-charge maintenance equity stays at or
    /// above `MM_req + notional * collection_maint_buffer_bps / 10_000`.
    ///
    /// The uncollected remainder is DROPPED (not deferred): the wrapper does not
    /// carry premium debt, and `last_premium_slot` still advances so the same
    /// interval is never re-charged. Dropping under stress is the conservative
    /// choice — it protects the buffer that backs every other account.
    ///
    /// Default `0` = permissive (no buffer floor), preserving the prior
    /// "charge up to available capital" behaviour. A production deployment
    /// SHOULD set a nonzero buffer (e.g. 50–200 bps) sized to the worst-case
    /// one-step price move so a single accrual can never tip an account into
    /// liquidation it would otherwise have survived.
    pub collection_maint_buffer_bps: u128,

    // ── WS2 Task 4: compliance guards (governance params, opt-in) ────────────
    /// Maximum tolerated divergence, in basis points, between a caller-supplied
    /// `oracle_price` and the engine's last accrued price (`last_oracle_price`).
    /// If `|oracle_price - last| * 10_000 / last > max_oracle_deviation_bps`,
    /// the extraction-sensitive op is rejected with `OracleDivergence`.
    ///
    /// Default `0` = guard DISABLED (permissive). This is intentional: the
    /// engine's own bounded-crank logic already clamps per-step price moves, so
    /// the default keeps existing flows working. A production deployment SHOULD
    /// set a bound (e.g. a few hundred bps) matched to the feed's expected jitter.
    ///
    // PRODUCTION NOTE: this is a coarse sanity clamp against gross divergence /
    // stale marks only. Full oracle SOURCING — choosing a trustworthy feed,
    // aggregating multiple sources, confidence-interval handling, TWAP/median
    // filtering — remains the integrator's responsibility per the README.
    pub max_oracle_deviation_bps: u64,
    /// Maximum tolerated staleness, in slots, of the engine's last market
    /// update (`current_slot - last_market_slot`) at the time an
    /// extraction-sensitive op is attempted. `0` = guard DISABLED (permissive).
    ///
    // PRODUCTION NOTE: a real deployment must additionally validate the freshness
    // of the EXTERNAL feed it sources `oracle_price` from; this guard only checks
    // that the engine's internal mark was cranked recently.
    pub max_oracle_staleness_slots: u64,
    /// When `true`, every wrapped op that exposes an `*_authorized` entrypoint
    /// requires the caller to present an authority matching the account's
    /// claimed `owner` (or the account being unclaimed, owner == `[0u8; 32]`).
    /// Default `false` = guard DISABLED (the plain entrypoints bypass the check).
    ///
    // PRODUCTION NOTE: this is only a hook. The engine NEVER reads `owner` for
    // any spec-normative decision (margin/liquidation/fees). Real authorization —
    // signature verification, PDA/seed checks, role policy — is the integrator's
    // responsibility; this field merely gives a place to wire the gate.
    pub require_authorization: bool,
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
    /// Monotonic accumulator of system_index_scaled × slots (funding pattern).
    pub cum_system_index: u128,
    /// Slot at which `cum_system_index` was last advanced.
    pub last_accrue_slot: u64,
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
            // Review #2: a zero volatility denominator silently collapses the
            // multiplier chain's `den` to 0, neutralizing the premium to
            // min_premium instead of erroring; a zero numerator zeroes it. Both
            // are misconfigurations — reject them like every other ratio param.
            || premium_params.volatility_mult_den == 0
            || premium_params.volatility_mult_num == 0
        {
            return Err(InsuredError::InvalidParams);
        }

        let engine = RiskEngine::new_with_market(risk_params, init_slot, init_oracle_price);

        Ok(Self {
            engine,
            pool: PremiumPool::new(),
            premium_params,
            account_premiums: [AccountPremiumState::new(); MAX_ACCOUNTS],
            cum_system_index: 0,
            last_accrue_slot: init_slot,
        })
    }

    pub fn compute_risk_index(&self, account_idx: usize) -> RiskIndex {
        if account_idx >= MAX_ACCOUNTS {
            return RiskIndex::neutral();
        }
        let notional = self.account_notional(account_idx);
        self.risk_index_with_notional(account_idx, notional)
    }

    /// Build the risk index reusing an already-computed `notional`, so the
    /// per-slot hot path doesn't call `account_notional` twice (review #3).
    fn risk_index_with_notional(&self, account_idx: usize, notional: u128) -> RiskIndex {
        debug_assert!(account_idx < MAX_ACCOUNTS);
        let long_oi = self.engine.oi_eff_long_q;
        let short_oi = self.engine.oi_eff_short_q;
        let vault = self.engine.vault.get();
        let oracle_price = self.engine.last_oracle_price;
        let pp = &self.premium_params;

        let pos = self.engine.accounts[account_idx].position_basis_q;
        // Task 1 RE-POINT: charge the MINORITY side, not the majority.
        //
        // The engine socializes a liquidation deficit onto the side OPPOSITE the
        // liquidated side: `enqueue_adl` (percolator.rs) computes
        // `opp = opposite_side(liq_side)` and shifts `K_opp` downward by
        // `delta_k_abs = ceil(D_rem * A_old * POS_SCALE / OI)`. So the cohort
        // that actually absorbs a socialized deficit is the counterparty to the
        // liquidated side. In the canonical tail (the crowded MAJORITY cascades
        // into liquidation when the market moves against it), the deficit lands
        // on the MINORITY counterparties. Therefore the minority side bears the
        // socialization risk and must be priced for it — the old code penalized
        // the majority, which is the side that is liquidating, not the side that
        // pays for it.
        //
        // We keep `crowding_multiplier`'s contract (it penalizes whichever side
        // we flag as the charged side) and flip the flag: pass `true` when this
        // account sits on the minority side. The ratio `majority/minority` still
        // measures the imbalance severity.
        let (majority_oi, minority_oi, is_charged_side) = if long_oi >= short_oi {
            // longs are the majority → shorts (pos < 0) are the charged minority
            (long_oi, short_oi, pos < 0)
        } else {
            // shorts are the majority → longs (pos > 0) are the charged minority
            (short_oi, long_oi, pos > 0)
        };

        let crowding = if pos == 0 {
            (MULT_SCALE, MULT_SCALE)
        } else {
            crowding_multiplier(
                majority_oi,
                minority_oi,
                is_charged_side,
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

        // Task 2: realized-volatility multiplier. No calibrated on-chain vol
        // source exists in this crate, so we pass the governance param straight
        // through. CALIBRATION REQUIRED: feed an oracle-realized-volatility
        // multiplier into `PremiumParams::volatility_mult_*` in production.
        let volatility = (pp.volatility_mult_num, pp.volatility_mult_den);

        // Task 3: leverage tail surcharge. Maintenance margin is read from the
        // engine's live RiskParams (`maintenance_margin_bps`), so the surcharge
        // tracks the actual maintenance limit `L_max = 10_000 / mm_bps`.
        // `notional` is supplied by the caller (computed once).
        let capital = self.engine.accounts[account_idx].capital.get();
        let leverage_tail = leverage_tail_surcharge(
            notional,
            capital,
            self.engine.params.maintenance_margin_bps as u128,
            pp.leverage_tail_threshold_bps,
            pp.leverage_tail_steepness,
        );

        RiskIndex {
            crowding,
            oi_vault,
            pool_health,
            volatility,
            leverage_tail,
        }
    }

    fn account_notional(&self, idx: usize) -> u128 {
        debug_assert!(idx < MAX_ACCOUNTS, "account_notional: idx out of range");
        let pos = self.engine.accounts[idx].position_basis_q;
        if pos == 0 {
            return 0;
        }
        let abs_pos = pos.unsigned_abs();
        abs_pos.saturating_mul(self.engine.last_oracle_price as u128) / POS_SCALE
    }

    // ====================================================================
    // Global system-risk accumulator (funding-style)
    // ====================================================================

    /// Current account-independent system index in MULT_SCALE units
    /// (oi_vault × pool_health × volatility). No account dimension.
    fn current_system_index(&self) -> u128 {
        let long_oi = self.engine.oi_eff_long_q;
        let short_oi = self.engine.oi_eff_short_q;
        let vault = self.engine.vault.get();
        let oracle_price = self.engine.last_oracle_price;
        let pp = &self.premium_params;

        let total_oi_q = long_oi.saturating_add(short_oi);
        let total_oi_notional = if oracle_price > 0 {
            total_oi_q.saturating_mul(oracle_price as u128) / POS_SCALE
        } else {
            0
        };
        let oi_vault = oi_vault_multiplier(
            total_oi_notional, vault,
            pp.oi_vault_floor_ratio_num, pp.oi_vault_floor_ratio_den,
            pp.oi_vault_cap_ratio_num, pp.oi_vault_cap_ratio_den, pp.oi_vault_mult_max,
        );
        let pool_health = pool_health_multiplier(
            self.pool.balance, total_oi_notional,
            pp.pool_health_low_num, pp.pool_health_low_den,
            pp.pool_health_high_num, pp.pool_health_high_den, pp.pool_health_mult_max,
        );
        let volatility = (pp.volatility_mult_num, pp.volatility_mult_den);
        crate::premium::compute_system_index_scaled(oi_vault, pool_health, volatility)
    }

    /// Advance the global system-risk accumulator to `now_slot`. Permissionless;
    /// a no-op when time does not advance. The keeper SHOULD call this each crank.
    pub fn accrue(&mut self, now_slot: u64) {
        if now_slot <= self.last_accrue_slot {
            return;
        }
        let dt = (now_slot - self.last_accrue_slot) as u128;
        let s = self.current_system_index();
        self.cum_system_index = self.cum_system_index.saturating_add(s.saturating_mul(dt));
        self.last_accrue_slot = now_slot;
    }

    // ====================================================================
    // WS2 Task 3: counter-cyclical collection cap
    // ====================================================================

    /// Cap a requested premium charge so it cannot push account `idx` below its
    /// maintenance-margin requirement plus the governance safety buffer.
    ///
    /// Returns the largest amount `<= requested` that leaves post-charge
    /// maintenance equity at or above
    /// `MM_req + notional * collection_maint_buffer_bps / 10_000`.
    ///
    /// Maintenance equity mirrors the engine exactly:
    ///   `Eq_maint_raw = capital + pnl - fee_debt`  (account_equity_maint_raw)
    ///   `MM_req       = max(notional * mm_bps / 10_000, min_nonzero_mm_req)`
    /// A premium charge only reduces `capital`, so it reduces `Eq_maint_raw`
    /// 1:1 (the engine routes the charge through capital first). The maximum
    /// chargeable without breaching the floor is therefore
    ///   `headroom = max(0, Eq_maint_raw - (MM_req + buffer))`.
    ///
    /// When `collection_maint_buffer_bps == 0` this returns `requested`
    /// unchanged (permissive default — no behaviour change).
    fn cap_premium_to_maint_buffer(&self, idx: usize, requested: u128) -> u128 {
        let buffer_bps = self.premium_params.collection_maint_buffer_bps;
        if buffer_bps == 0 || requested == 0 {
            return requested;
        }

        let acct = &self.engine.accounts[idx];
        let eq_maint_raw = self.engine.account_equity_maint_raw(acct);
        if eq_maint_raw <= 0 {
            // Already at/below zero maintenance equity — charge nothing more.
            return 0;
        }
        let eq_maint = eq_maint_raw as u128;

        let notional = self.account_notional(idx);
        let proportional = notional
            .saturating_mul(self.engine.params.maintenance_margin_bps as u128)
            / 10_000;
        let mm_req = core::cmp::max(proportional, self.engine.params.min_nonzero_mm_req);
        let buffer = notional.saturating_mul(buffer_bps) / 10_000;
        let floor = mm_req.saturating_add(buffer);

        let headroom = eq_maint.saturating_sub(floor);
        core::cmp::min(requested, headroom)
    }

    // ====================================================================
    // WS2 Task 4: compliance guards (oracle sanity + authorization)
    // ====================================================================

    /// Reject a caller-supplied `oracle_price` that is stale or diverges beyond
    /// the configured bound from the engine's last accrued price. No-op (Ok)
    /// when both guards are disabled (the permissive default), so existing
    /// callers are unaffected.
    fn check_oracle_sanity(&self, oracle_price: u64, now_slot: u64) -> crate::Result<()> {
        let pp = &self.premium_params;

        // Staleness: how long since the engine last cranked its mark.
        if pp.max_oracle_staleness_slots > 0 {
            let last = self.engine.last_market_slot;
            // Review #6: clamp the time reference to the engine's own
            // `current_slot` so a caller cannot UNDERSTATE staleness by passing a
            // small `now_slot`. (The engine independently rejects
            // `now_slot < current_slot` on the actual op, but this guard runs
            // before that and must be robust on its own.)
            let now = now_slot.max(self.engine.current_slot);
            let staleness = now.saturating_sub(last);
            if staleness > pp.max_oracle_staleness_slots {
                return Err(InsuredError::OracleDivergence);
            }
        }

        // Divergence: |oracle_price - last| / last in bps.
        if pp.max_oracle_deviation_bps > 0 {
            let last = self.engine.last_oracle_price as u128;
            if last == 0 {
                // No reference price yet — cannot bound divergence; reject
                // conservatively when the guard is enabled.
                return Err(InsuredError::OracleDivergence);
            }
            let p = oracle_price as u128;
            let diff = p.abs_diff(last);
            let diff_bps = diff.saturating_mul(10_000) / last;
            if diff_bps > pp.max_oracle_deviation_bps as u128 {
                return Err(InsuredError::OracleDivergence);
            }
        }
        Ok(())
    }

    /// Enforce the optional authorization hook: when `require_authorization` is
    /// set, the caller authority must match the account's claimed `owner`, or
    /// the account must be unclaimed (`owner == [0u8; 32]`). No-op (Ok) when the
    /// guard is disabled.
    ///
    /// `caller == None` means no authority was presented (the plain, non-
    /// `*_authorized` entrypoints). With the guard enabled, a claimed account
    /// then rejects — the integrator MUST route claimed-account calls through
    /// the `*_authorized` entrypoints.
    ///
    // PRODUCTION NOTE: this is a structural hook only. The engine never reads
    // `owner` for spec-normative decisions; binding `owner` to a real signer
    // (signature/PDA verification) is the integrator's responsibility.
    fn check_authorization(&self, idx: usize, caller: Option<&[u8; 32]>) -> crate::Result<()> {
        if !self.premium_params.require_authorization {
            return Ok(());
        }
        let owner = self.engine.accounts[idx].owner;
        if owner == [0u8; 32] {
            // Unclaimed account: nothing to authorize against.
            return Ok(());
        }
        match caller {
            Some(c) if &owner == c => Ok(()),
            _ => Err(InsuredError::Unauthorized),
        }
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
        // Review #4: a flat account (notional == 0) owes no premium
        // (compute_premium_per_slot returns 0 for it anyway) — skip building the
        // full risk index and premium entirely on the per-slot hot path.
        if notional == 0 {
            self.account_premiums[i].last_premium_slot = now_slot;
            return Ok(0);
        }
        let capital = self.engine.accounts[i].capital.get();
        // Review #3: reuse `notional` instead of recomputing it in the index.
        let risk_idx = self.risk_index_with_notional(i, notional);

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
            // WS2 Task 3: cap the capital-charging portion so collection cannot
            // push the account below its maintenance + safety buffer. The
            // prepaid drawdown above is pure wrapper accounting (no capital
            // touched), so only this leg can erode the liquidation buffer. The
            // uncollected remainder beyond the cap is DROPPED (see field doc).
            let to_charge = self.cap_premium_to_maint_buffer(i, remaining);

            if to_charge > 0 {
                // The engine caps the fee at the account's available capital and
                // silently drops any excess (charge_fee_to_insurance), returning
                // Ok even when capital is insufficient. Measure the actual
                // insurance-fund delta and record only that — recording the
                // requested amount would over-state the pool's claim on the fund.
                let before = self.engine.insurance_fund.balance.get();
                self.engine
                    .charge_account_fee_not_atomic(idx, to_charge, now_slot)
                    .map_err(InsuredError::Risk)?;
                let after = self.engine.insurance_fund.balance.get();
                let actually_collected = after.saturating_sub(before);
                if actually_collected > 0 {
                    self.pool.record_collection(actually_collected)?;
                    collected += actually_collected;
                }
            }
        }

        self.account_premiums[i].last_premium_slot = now_slot;
        Ok(collected)
    }

    /// Reconcile the premium pool's claim against the real insurance-fund
    /// balance, attributing any shortfall to deficit coverage.
    ///
    /// INVARIANT (review #1): this attribution is only correct if the wrapper is
    /// the SOLE mutator of the insurance fund. The pool is an accounting shadow
    /// — `reconcile_with_insurance_balance` treats *any* drop in the fund below
    /// the pool's recorded claim as premium-funded deficit coverage. If an
    /// integrator drains the fund through the engine's governance entrypoints
    /// (`withdraw_insurance_not_atomic` / `withdraw_live_insurance_not_atomic` /
    /// `withdraw_resolved_insurance_not_atomic`) while bypassing this wrapper,
    /// that withdrawal is mis-booked as a phantom payout, polluting
    /// `total_paid_out` (the loss-ratio signal `calibrate_base_rate` consumes).
    /// A production integration must therefore either route all insurance-fund
    /// mutations through the wrapper, or replace this inferential reconcile with
    /// an engine-exposed premium-attributable-consumption counter.
    pub fn reconcile_pool(&mut self) {
        let insurance = self.engine.insurance_fund.balance.get();
        self.pool.reconcile_with_insurance_balance(insurance);
    }

    // ====================================================================
    // Wrapped Percolator operations
    // ====================================================================

    /// Deposit without presenting a caller authority. With
    /// `require_authorization` enabled this is rejected for claimed accounts —
    /// use [`deposit_authorized`](Self::deposit_authorized).
    pub fn deposit(
        &mut self,
        idx: u16,
        amount: u128,
        now_slot: u64,
    ) -> crate::Result<()> {
        self.deposit_inner(idx, amount, now_slot, None)
    }

    /// Deposit on behalf of `caller`. The authority must match the account's
    /// claimed owner when `require_authorization` is enabled.
    pub fn deposit_authorized(
        &mut self,
        idx: u16,
        amount: u128,
        now_slot: u64,
        caller: [u8; 32],
    ) -> crate::Result<()> {
        self.deposit_inner(idx, amount, now_slot, Some(&caller))
    }

    fn deposit_inner(
        &mut self,
        idx: u16,
        amount: u128,
        now_slot: u64,
        caller: Option<&[u8; 32]>,
    ) -> crate::Result<()> {
        let i = idx as usize;
        if i < MAX_ACCOUNTS {
            // WS2 Task 4: authorization hook (no-op when guard disabled).
            self.check_authorization(i, caller)?;
        }
        if i < MAX_ACCOUNTS && self.account_premiums[i].is_active {
            // WS2 Task 2: propagate collection failure instead of swallowing it.
            self.collect_accrued_premium(idx, now_slot)?;
        }

        self.engine
            .deposit_not_atomic(idx, amount, now_slot)
            .map_err(InsuredError::Risk)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
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

        // WS2 Task 4: oracle sanity guard (no-op when both bounds are 0).
        // execute_trade is extraction-sensitive — reject a stale/divergent mark
        // before it can drive PnL or position changes.
        self.check_oracle_sanity(oracle_price, now_slot)?;

        if ai < MAX_ACCOUNTS && self.account_premiums[ai].is_active {
            // WS2 Task 2: propagate collection failure instead of swallowing it.
            self.collect_accrued_premium(a, now_slot)?;
        }
        if bi < MAX_ACCOUNTS && self.account_premiums[bi].is_active {
            self.collect_accrued_premium(b, now_slot)?;
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
        // Review #3: reuse `notional` instead of recomputing it in the index.
        let risk_idx = self.risk_index_with_notional(i, notional);

        let rate = compute_premium_per_slot(
            notional,
            capital,
            self.premium_params.base_rate_per_slot,
            &risk_idx,
            self.premium_params.min_premium_per_slot,
        );

        let commitment = rate.saturating_mul(self.premium_params.min_commitment_slots as u128);

        // WS2 Task 3: the upfront commitment is the single largest charge and is
        // taken right after the position opens, when the buffer matters most.
        // Cap it to the maintenance + safety floor so opening a position can
        // never pre-emptively erode the liquidation buffer the fund relies on.
        let to_charge = self.cap_premium_to_maint_buffer(i, commitment);

        let mut charged = 0u128;
        if to_charge > 0 {
            // Record only what actually reached the insurance fund: the engine
            // caps the charge at available capital and drops the rest (see
            // collect_accrued_premium). prepaid_premium must reflect the real
            // amount paid, not the requested commitment.
            let before = self.engine.insurance_fund.balance.get();
            self.engine
                .charge_account_fee_not_atomic(idx, to_charge, now_slot)
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

    /// Withdraw without presenting a caller authority. With
    /// `require_authorization` enabled this is rejected for claimed accounts —
    /// use [`withdraw_authorized`](Self::withdraw_authorized).
    #[allow(clippy::too_many_arguments)]
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
        self.withdraw_inner(
            idx, amount, oracle_price, now_slot, funding_rate_e9,
            admit_h_min, admit_h_max, admit_h_max_consumption_threshold_bps_opt, None,
        )
    }

    /// Withdraw on behalf of `caller`. Withdrawal is the prime extraction op, so
    /// it is gated by both the authorization hook and the oracle-sanity guard.
    #[allow(clippy::too_many_arguments)]
    pub fn withdraw_authorized(
        &mut self,
        idx: u16,
        amount: u128,
        oracle_price: u64,
        now_slot: u64,
        funding_rate_e9: i128,
        admit_h_min: u64,
        admit_h_max: u64,
        admit_h_max_consumption_threshold_bps_opt: Option<u128>,
        caller: [u8; 32],
    ) -> crate::Result<()> {
        self.withdraw_inner(
            idx, amount, oracle_price, now_slot, funding_rate_e9,
            admit_h_min, admit_h_max, admit_h_max_consumption_threshold_bps_opt, Some(&caller),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn withdraw_inner(
        &mut self,
        idx: u16,
        amount: u128,
        oracle_price: u64,
        now_slot: u64,
        funding_rate_e9: i128,
        admit_h_min: u64,
        admit_h_max: u64,
        admit_h_max_consumption_threshold_bps_opt: Option<u128>,
        caller: Option<&[u8; 32]>,
    ) -> crate::Result<()> {
        let i = idx as usize;
        if i < MAX_ACCOUNTS {
            // WS2 Task 4: authorization + oracle sanity. Withdraw extracts
            // capital, so it is the canonical extraction-sensitive action the
            // README says to reject during oracle divergence.
            self.check_authorization(i, caller)?;
        }
        self.check_oracle_sanity(oracle_price, now_slot)?;

        if i < MAX_ACCOUNTS && self.account_premiums[i].is_active {
            // WS2 Task 2: propagate collection failure instead of swallowing it.
            self.collect_accrued_premium(idx, now_slot)?;
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

    #[allow(clippy::too_many_arguments)]
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
        // WS2 Task 4: the oracle-sanity guard is INTENTIONALLY NOT applied to
        // liquidation. Liquidation is a risk-REDUCING keeper action, not an
        // owner extraction; blocking it during a fast/divergent mark would
        // strand under-collateralized accounts and worsen fund solvency — the
        // opposite of the guard's intent. Authorization is likewise not gated
        // (liquidation is permissionless by design). The engine's own bounded-
        // crank logic still clamps the price move applied inside the call.
        if i < MAX_ACCOUNTS && self.account_premiums[i].is_active {
            // WS2 Task 2: propagate collection failure instead of swallowing it.
            self.collect_accrued_premium(idx, now_slot)?;
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
