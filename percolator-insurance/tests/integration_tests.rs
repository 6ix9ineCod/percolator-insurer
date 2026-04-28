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
