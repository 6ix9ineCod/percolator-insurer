use percolator::{LiquidationPolicy, RiskParams, U128, MAX_ACCOUNTS, POS_SCALE};
use percolator_insurance::wrapper::{AccountPremiumState, InsuredRiskEngine, PremiumParams};
use percolator_insurance::MULT_SCALE;

fn test_risk_params() -> RiskParams {
    RiskParams {
        maintenance_margin_bps: 500,
        initial_margin_bps: 1000,
        trading_fee_bps: 10,
        max_accounts: MAX_ACCOUNTS as u64,
        liquidation_fee_bps: 100,
        liquidation_fee_cap: U128::new(1_000_000),
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
    }
}

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
        // Task 2: neutral volatility multiplier (no calibrated source).
        volatility_mult_num: MULT_SCALE,
        volatility_mult_den: MULT_SCALE,
        // Task 3: tail surcharge onset at 80% of L_max, steepness 3.0x.
        leverage_tail_threshold_bps: 8000,
        leverage_tail_steepness: 3000,
        // WS2 Task 3: collection cap disabled by default (permissive).
        collection_maint_buffer_bps: 0,
        // WS2 Task 4: oracle/auth guards disabled by default (permissive).
        max_oracle_deviation_bps: 0,
        max_oracle_staleness_slots: 0,
        require_authorization: false,
    }
}

fn make_size_q(quantity: i64) -> i128 {
    let abs_qty = (quantity as i128).unsigned_abs();
    let scaled = abs_qty.checked_mul(POS_SCALE).expect("make_size_q overflow");
    if quantity < 0 {
        -(scaled as i128)
    } else {
        scaled as i128
    }
}

fn setup_engine() -> InsuredRiskEngine {
    let rp = test_risk_params();
    let pp = test_premium_params();
    let oracle = 1000u64;
    let slot = 1u64;

    let mut eng = InsuredRiskEngine::new(rp, pp, slot, oracle).unwrap();

    // Deposit into accounts 0 and 1 (materializes them)
    // 10M each — large enough for commitment premiums
    eng.deposit(0, 10_000_000, slot).unwrap();
    eng.deposit(1, 10_000_000, slot).unwrap();

    // Initial crank so trades pass the freshness check
    eng.engine
        .keeper_crank_not_atomic(
            slot,
            oracle,
            &[] as &[(u16, Option<LiquidationPolicy>)],
            64,
            0i128,
            0,
            100,
            None,
            0,
        )
        .unwrap();

    eng
}

// ====================================================================
// Task 6: Initialization tests
// ====================================================================

#[test]
fn test_wrapper_new() {
    let rp = test_risk_params();
    let pp = test_premium_params();
    let engine = InsuredRiskEngine::new(rp, pp, 1, 1000).unwrap();
    assert_eq!(engine.pool.balance, 0);
    assert!(engine.pool.check_invariants());
    assert!(engine.engine.check_conservation());
}

#[test]
fn test_wrapper_invalid_params_zero_den() {
    let rp = test_risk_params();
    let mut pp = test_premium_params();
    pp.leverage_exponent_den = 0;
    assert!(InsuredRiskEngine::new(rp, pp, 1, 1000).is_err());
}

#[test]
fn test_wrapper_initial_premium_state() {
    let rp = test_risk_params();
    let pp = test_premium_params();
    let engine = InsuredRiskEngine::new(rp, pp, 1, 1000).unwrap();
    for i in 0..MAX_ACCOUNTS {
        assert_eq!(engine.account_premiums[i], AccountPremiumState::new());
        assert!(!engine.account_premiums[i].is_active);
    }
}

#[test]
fn test_compute_risk_index_no_positions() {
    let engine = setup_engine();
    let idx = engine.compute_risk_index(0);
    assert_eq!(idx.crowding, (MULT_SCALE, MULT_SCALE));
}

// ====================================================================
// Task 1: crowding re-point — the MINORITY side is charged, not the majority
// ====================================================================

#[test]
fn test_crowding_charges_minority_not_majority() {
    // Engine socializes deficits onto the side OPPOSITE the liquidated side
    // (enqueue_adl shifts K_opp). The crowded majority is the side that
    // cascades into liquidation; the deficit lands on the minority. So the
    // minority must pay the crowding penalty and the majority must not.
    let mut engine = setup_engine();

    // Build a 5x-imbalanced book directly: longs dominate.
    engine.engine.oi_eff_long_q = 5_000_000;
    engine.engine.oi_eff_short_q = 1_000_000;

    // Account 0 = LONG (majority side). Account 1 = SHORT (minority side).
    engine.engine.accounts[0].position_basis_q = 1_000; // pos > 0 → long
    engine.engine.accounts[1].position_basis_q = -1_000; // pos < 0 → short

    let majority_idx = engine.compute_risk_index(0); // long → majority
    let minority_idx = engine.compute_risk_index(1); // short → minority

    // Majority (long) is NOT charged → neutral.
    assert_eq!(
        majority_idx.crowding,
        (MULT_SCALE, MULT_SCALE),
        "majority (crowded) side must NOT be penalized post re-point"
    );
    // Minority (short) IS charged → penalty above 1.0x.
    assert!(
        minority_idx.crowding.0 > MULT_SCALE,
        "minority side must bear the crowding penalty (socialization risk), got {:?}",
        minority_idx.crowding
    );
}

// ====================================================================
// Task 3: leverage tail surcharge wires through compute_risk_index
// ====================================================================

#[test]
fn test_leverage_tail_surcharge_wired() {
    // maintenance_margin_bps = 500 → L_max = 20x, onset 16x (threshold 8000).
    // Give account 0 a high-leverage position near the boundary.
    let mut engine = setup_engine();

    // notional ≈ pos * oracle / POS_SCALE; capital is the account's capital.
    // Make leverage ~18x: capital 10M, notional ~180M.
    // position_basis_q = notional * POS_SCALE / oracle.
    let oracle = 1000u128;
    let target_notional = 180_000_000u128;
    let pos = (target_notional * POS_SCALE / oracle) as i128;
    engine.engine.accounts[0].position_basis_q = pos;

    let idx = engine.compute_risk_index(0);
    // ~18x is between onset (16x) and L_max (20x) → surcharge > 1.0x.
    assert!(
        idx.leverage_tail.0 > MULT_SCALE,
        "near-maintenance leverage must trigger the tail surcharge, got {:?}",
        idx.leverage_tail
    );

    // A low-leverage account (1) with no position → neutral surcharge.
    let idx_flat = engine.compute_risk_index(1);
    assert_eq!(
        idx_flat.leverage_tail,
        (MULT_SCALE, MULT_SCALE),
        "flat account must have neutral tail surcharge"
    );
}

// ====================================================================
// Task 7: Wrapped operations tests
// ====================================================================

#[test]
fn test_deposit() {
    let rp = test_risk_params();
    let pp = test_premium_params();
    let mut engine = InsuredRiskEngine::new(rp, pp, 1, 1000).unwrap();
    engine.deposit(0, 100_000, 10).unwrap();
    assert_eq!(engine.engine.accounts[0].capital.get(), 100_000);
    assert!(engine.engine.check_conservation());
}

#[test]
fn test_full_lifecycle() {
    let mut engine = setup_engine();
    let oracle = 1000u64;
    let size_q = make_size_q(10);

    engine
        .execute_trade(0, 1, oracle, 2, size_q, oracle, 0, 0, 100, None)
        .unwrap();

    assert!(engine.account_premiums[0].is_active);
    assert!(engine.account_premiums[1].is_active);
    assert!(engine.engine.check_conservation());
    assert!(engine.pool.check_invariants());

    let pool_collected = engine.pool.total_collected;
    assert!(pool_collected > 0, "commitment premiums must be collected");
}

#[test]
fn test_commitment_charged_on_trade() {
    let mut engine = setup_engine();
    let oracle = 1000u64;
    let pp = test_premium_params();

    engine
        .execute_trade(0, 1, oracle, 2, make_size_q(10), oracle, 0, 0, 100, None)
        .unwrap();

    let commitment_a = engine.account_premiums[0].prepaid_premium;
    let commitment_b = engine.account_premiums[1].prepaid_premium;
    assert!(commitment_a > 0, "account A commitment must be charged");
    assert!(commitment_b > 0, "account B commitment must be charged");
    assert_eq!(
        engine.account_premiums[0].commitment_end_slot,
        2 + pp.min_commitment_slots
    );
    assert!(engine.engine.check_conservation());
}

#[test]
fn test_close_position_deactivates_premium() {
    let mut engine = setup_engine();
    let oracle = 1000u64;

    engine
        .execute_trade(0, 1, oracle, 2, make_size_q(10), oracle, 0, 0, 100, None)
        .unwrap();
    assert!(engine.account_premiums[0].is_active);

    // Close: swap a/b to reverse direction (size_q is always positive)
    engine
        .execute_trade(1, 0, oracle, 2, make_size_q(10), oracle, 0, 0, 100, None)
        .unwrap();

    assert!(!engine.account_premiums[0].is_active);
    assert!(!engine.account_premiums[1].is_active);
    assert!(engine.pool.total_collected > 0);
    assert!(engine.engine.check_conservation());
}

#[test]
fn test_collect_premium_no_position() {
    let mut engine = setup_engine();
    let collected = engine.collect_accrued_premium(0, 1000).unwrap();
    assert_eq!(collected, 0);
}

#[test]
fn test_collect_premium_after_trade() {
    let mut engine = setup_engine();
    let oracle = 1000u64;

    engine
        .execute_trade(0, 1, oracle, 2, make_size_q(10), oracle, 0, 0, 100, None)
        .unwrap();

    let _pool_before = engine.pool.total_collected;

    // Advance 50 slots and collect
    let collected = engine.collect_accrued_premium(0, 52).unwrap();
    assert!(collected > 0, "premium must be owed after time passes");
    assert!(engine.pool.check_invariants());
    assert!(engine.engine.check_conservation());
}

#[test]
fn test_reconcile_pool() {
    let mut engine = setup_engine();
    let oracle = 1000u64;

    engine
        .execute_trade(0, 1, oracle, 2, make_size_q(10), oracle, 0, 0, 100, None)
        .unwrap();

    let insurance = engine.engine.insurance_fund.balance.get();
    assert!(
        engine.pool.balance <= insurance || insurance == 0,
        "pool claim must not exceed insurance fund"
    );
    assert!(engine.pool.check_invariants());
}

#[test]
fn test_pool_records_only_actual_collection_when_capital_insufficient() {
    // The engine caps a fee at the account's available capital and silently
    // drops any excess (charge_fee_to_insurance), returning Ok even when the
    // account cannot pay in full. The wrapper must therefore record into the
    // pool only what ACTUALLY reached the insurance fund — never the requested
    // amount. Over-recording inflates the pool's claim above the fund balance,
    // which reconcile_pool then mistakes for a deficit payout: total_paid_out
    // rises even though no loss occurred.
    let rp = test_risk_params();
    let pp = test_premium_params(); // min_commitment_slots = 216_000
    let oracle = 1000u64;
    let slot = 1u64;

    let mut engine = InsuredRiskEngine::new(rp, pp, slot, oracle).unwrap();

    // Counterparty is well funded.
    engine.deposit(1, 10_000_000, slot).unwrap();
    // Account 0 is deliberately underfunded: enough margin to open the
    // position, but far less than the 216_000 commitment premium.
    engine.deposit(0, 100_000, slot).unwrap();

    engine
        .engine
        .keeper_crank_not_atomic(
            slot,
            oracle,
            &[] as &[(u16, Option<LiquidationPolicy>)],
            64,
            0i128,
            0,
            100,
            None,
            0,
        )
        .unwrap();

    // Opening the position charges the commitment premium to both accounts.
    // Account 0 cannot cover it, so only part reaches the fund.
    engine
        .execute_trade(0, 1, oracle, 2, make_size_q(10), oracle, 0, 0, 100, None)
        .unwrap();

    // No liquidation, no socialized loss has happened — only premium
    // collection — so the pool must not have booked any payout.
    assert_eq!(
        engine.pool.total_paid_out, 0,
        "pool booked a phantom payout (total_paid_out={}) — premiums were over-recorded \
         beyond what the underfunded account actually paid into the insurance fund",
        engine.pool.total_paid_out
    );
    assert!(engine.pool.balance <= engine.engine.insurance_fund.balance.get());
    assert!(engine.pool.check_invariants());
    assert!(engine.engine.check_conservation());
}

// ====================================================================
// WS2 Task 2: discarded-result policy — collection failures before a
// wrapped op are propagated, not silently swallowed.
// ====================================================================

#[test]
fn test_collection_failure_propagates_from_deposit() {
    // A genuine collection failure (an Err from the engine, NOT the capped-fee
    // Ok path) must ABORT the wrapped op and surface to the caller rather than
    // being swallowed by `let _ = ...`. We isolate the collection error from the
    // wrapped op: the fee charge rejects fee_abs > MAX_PROTOCOL_FEE_ABS (10^36)
    // with RiskError::Overflow, while deposit_not_atomic at the SAME, fresh slot
    // succeeds. So if collection errors are propagated the deposit fails; if
    // they were swallowed the deposit would succeed.
    let rp = test_risk_params();
    let mut pp = test_premium_params();
    // Brutal flat rate so premium_owed over a modest interval exceeds 10^36.
    // Cap guard OFF (default 0) so the huge charge actually reaches the engine.
    pp.base_rate_per_slot = 1_000_000_000_000_000_000_000_000_000_000_000; // 10^33
    pp.min_premium_per_slot = 1_000_000_000_000_000_000_000_000_000_000_000; // 10^33
    pp.min_commitment_slots = 1; // tiny commitment so the account survives open
    let oracle = 1000u64;
    let slot = 1u64;

    let mut engine = InsuredRiskEngine::new(rp, pp, slot, oracle).unwrap();
    // Fund both accounts richly so the position opens and the deposit is valid.
    engine.deposit(0, 100_000_000, slot).unwrap();
    engine.deposit(1, 100_000_000, slot).unwrap();
    engine
        .engine
        .keeper_crank_not_atomic(
            slot,
            oracle,
            &[] as &[(u16, Option<LiquidationPolicy>)],
            64,
            0i128,
            0,
            100,
            None,
            0,
        )
        .unwrap();

    // Open at slot 2 → last_premium_slot[0] = 2.
    engine
        .execute_trade(0, 1, oracle, 2, make_size_q(10), oracle, 0, 0, 100, None)
        .unwrap();
    assert!(engine.account_premiums[0].is_active);

    // Crank the engine (and last_market_slot) forward to slot 50 so the next
    // op at slot 50 is itself valid, but [2,50] of premium at 10^33/slot is
    // ~4.8 * 10^34 ... still below 10^36. Push to a slot where rate*dt > 10^36:
    // dt must exceed 1000 slots. Crank to 1100, then deposit at 1100.
    for s in [50u64, 200, 600, 1100] {
        engine
            .engine
            .keeper_crank_not_atomic(
                s,
                oracle,
                &[] as &[(u16, Option<LiquidationPolicy>)],
                64,
                0i128,
                0,
                100,
                None,
                0,
            )
            .unwrap();
    }

    // Deposit at slot 1100: collection owes ~1098 * 10^33 ≈ 1.1*10^36 > 10^36,
    // so charge_account_fee_not_atomic returns Err(Overflow). The deposit's own
    // call at slot 1100 (last_market_slot == 1100) would otherwise succeed.
    let res = engine.deposit(0, 1, 1100);
    assert!(
        res.is_err(),
        "collection failure before deposit must propagate (got Ok — error was swallowed)"
    );
}

// ====================================================================
// WS2 Task 3: counter-cyclical collection cap — a stressed, near-maintenance
// account is never charged below its maintenance + safety buffer.
// ====================================================================

#[test]
fn test_collection_cap_respects_maintenance_buffer() {
    let rp = test_risk_params();
    let mut pp = test_premium_params();
    // Demand a 200-bps-of-notional safety buffer above maintenance margin.
    pp.collection_maint_buffer_bps = 200;
    // Make premiums brutally expensive so an uncapped collection would drain
    // the account well below maintenance.
    pp.base_rate_per_slot = 1_000_000;
    pp.min_premium_per_slot = 1_000_000;
    let oracle = 1000u64;
    let slot = 1u64;

    let mut engine = InsuredRiskEngine::new(rp, pp, slot, oracle).unwrap();
    engine.deposit(1, 100_000_000, slot).unwrap();
    engine.deposit(0, 1_000_000, slot).unwrap();
    engine
        .engine
        .keeper_crank_not_atomic(
            slot,
            oracle,
            &[] as &[(u16, Option<LiquidationPolicy>)],
            64,
            0i128,
            0,
            100,
            None,
            0,
        )
        .unwrap();

    // Open a position; commitment charge is now capped by the buffer guard.
    engine
        .execute_trade(0, 1, oracle, 2, make_size_q(10), oracle, 0, 0, 100, None)
        .unwrap();

    // Advance many slots so an uncapped accrual would demand more than the
    // account's whole remaining capital, then collect.
    engine.collect_accrued_premium(0, slot + 100_000).unwrap();

    // The account must still be above its maintenance + safety floor.
    let acct = engine.engine.accounts[0];
    let eq_maint = engine.engine.account_equity_maint_raw(&acct);
    let notional = {
        let pos = acct.position_basis_q.unsigned_abs();
        pos.saturating_mul(engine.engine.last_oracle_price as u128) / POS_SCALE
    };
    let mm_req = core::cmp::max(
        notional * engine.engine.params.maintenance_margin_bps as u128 / 10_000,
        engine.engine.params.min_nonzero_mm_req,
    );
    let buffer = notional * 200 / 10_000;
    let floor = (mm_req + buffer) as i128;
    assert!(
        eq_maint >= floor,
        "collection drove equity {} below maintenance+buffer floor {}",
        eq_maint,
        floor
    );
    assert!(engine.engine.check_conservation());
    assert!(engine.pool.check_invariants());
}

// ====================================================================
// WS2 Task 4: compliance guards — oracle sanity + authorization hook.
// ====================================================================

#[test]
fn test_oracle_deviation_guard_rejects_divergent_price() {
    let rp = test_risk_params();
    let mut pp = test_premium_params();
    // Reject any oracle that diverges > 1% from the engine's last price.
    pp.max_oracle_deviation_bps = 100;
    let oracle = 1000u64;
    let slot = 1u64;

    let mut engine = InsuredRiskEngine::new(rp, pp, slot, oracle).unwrap();
    engine.deposit(0, 10_000_000, slot).unwrap();
    engine.deposit(1, 10_000_000, slot).unwrap();
    engine
        .engine
        .keeper_crank_not_atomic(
            slot,
            oracle,
            &[] as &[(u16, Option<LiquidationPolicy>)],
            64,
            0i128,
            0,
            100,
            None,
            0,
        )
        .unwrap();

    // last_oracle_price is ~1000; a price of 2000 diverges 100% >> 1%.
    let res = engine.execute_trade(0, 1, 2000, 2, make_size_q(10), 2000, 0, 0, 100, None);
    assert!(
        matches!(res, Err(percolator_insurance::InsuredError::OracleDivergence)),
        "divergent oracle must be rejected, got {:?}",
        res
    );

    // A price within 1% (e.g. 1005) must be accepted.
    let ok = engine.execute_trade(0, 1, 1005, 2, make_size_q(10), 1005, 0, 0, 100, None);
    assert!(ok.is_ok(), "in-bound oracle must pass, got {:?}", ok);
}

#[test]
fn test_authorization_guard_rejects_unauthorized_caller() {
    let rp = test_risk_params();
    let mut pp = test_premium_params();
    pp.require_authorization = true;
    let oracle = 1000u64;
    let slot = 1u64;

    let mut engine = InsuredRiskEngine::new(rp, pp, slot, oracle).unwrap();
    // Materialize and claim account 0 to owner [7u8;32].
    engine.deposit_authorized(0, 10_000_000, slot, [7u8; 32]).unwrap();
    engine.engine.set_owner(0, [7u8; 32]).unwrap();

    // Authorized caller matches the owner → ok.
    let ok = engine.deposit_authorized(0, 1, slot, [7u8; 32]);
    assert!(ok.is_ok(), "authorized caller must pass, got {:?}", ok);

    // Wrong authority → rejected.
    let bad = engine.deposit_authorized(0, 1, slot, [9u8; 32]);
    assert!(
        matches!(bad, Err(percolator_insurance::InsuredError::Unauthorized)),
        "unauthorized caller must be rejected, got {:?}",
        bad
    );
}

#[test]
fn test_wrapper_rejects_zero_volatility_den() {
    // Review #2: volatility_mult_den == 0 silently neutralizes the premium to
    // min_premium (den collapses in the multiplier chain) instead of erroring.
    // new() must reject it like every other denominator param.
    let rp = test_risk_params();
    let mut pp = test_premium_params();
    pp.volatility_mult_den = 0;
    assert!(
        InsuredRiskEngine::new(rp, pp, 1, 1000).is_err(),
        "zero volatility_mult_den must be rejected at construction"
    );
}

#[test]
fn test_activate_seeds_snapshot_and_leverage() {
    let mut engine = setup_engine();
    let oracle = 1000u64;
    engine.accrue(5); // advance the global accumulator first
    engine
        .execute_trade(0, 1, oracle, 6, make_size_q(10), oracle, 0, 0, 100, None)
        .unwrap();
    assert_eq!(
        engine.account_premiums[0].cum_system_snapshot,
        engine.cum_system_index
    );
    assert!(engine.account_premiums[0].last_leverage_factor >= MULT_SCALE,
        "leverage factor must be >= 1.0");
}

#[test]
fn test_accrue_global_is_monotonic_and_advances() {
    let mut engine = setup_engine();
    let start = engine.cum_system_index;
    engine.accrue(10);
    let after = engine.cum_system_index;
    assert!(after >= start, "accumulator must be monotonic");
    assert_eq!(engine.last_accrue_slot, 10);
    let frozen = engine.cum_system_index;
    engine.accrue(10);
    assert_eq!(engine.cum_system_index, frozen, "no-op when slot does not advance");
    engine.accrue(5);
    assert_eq!(engine.cum_system_index, frozen, "no-op when slot goes backward");
}
