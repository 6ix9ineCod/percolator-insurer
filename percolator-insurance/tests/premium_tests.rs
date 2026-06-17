use percolator_insurance::premium::{
    isqrt, inth_root, leverage_multiplier, compute_premium_per_slot,
    leverage_tail_surcharge, calibrate_base_rate,
};
use percolator_insurance::{MULT_SCALE, PREMIUM_SCALE};
use percolator_insurance::risk_index::RiskIndex;

// ============================================================================
// isqrt tests
// ============================================================================

#[test]
fn test_isqrt_zero() {
    assert_eq!(isqrt(0), 0);
}

#[test]
fn test_isqrt_one() {
    assert_eq!(isqrt(1), 1);
}

#[test]
fn test_isqrt_perfect_squares() {
    assert_eq!(isqrt(4), 2);
    assert_eq!(isqrt(9), 3);
    assert_eq!(isqrt(16), 4);
    assert_eq!(isqrt(100), 10);
    assert_eq!(isqrt(1_000_000), 1_000);
    assert_eq!(isqrt(1_000_000_000_000), 1_000_000);
}

#[test]
fn test_isqrt_non_perfect() {
    assert_eq!(isqrt(2), 1);
    assert_eq!(isqrt(3), 1);
    assert_eq!(isqrt(5), 2);
    assert_eq!(isqrt(8), 2);
    assert_eq!(isqrt(10), 3);
}

#[test]
fn test_isqrt_large() {
    let r = isqrt(u128::MAX);
    // r should be floor(sqrt(u128::MAX))
    assert!(r * r <= u128::MAX, "r*r should not exceed u128::MAX");
    // (r+1)^2 must overflow u128 — floor(sqrt(u128::MAX)) == u64::MAX,
    // and (u64::MAX + 1)^2 = 2^128 which overflows u128.
    let r1 = r + 1;
    assert!(
        r1.checked_mul(r1).is_none(),
        "Expected (r+1)^2 to overflow u128::MAX, r={}",
        r
    );
}

// ============================================================================
// inth_root tests
// ============================================================================

#[test]
fn test_inth_root_k1() {
    assert_eq!(inth_root(0, 1), 0);
    assert_eq!(inth_root(1, 1), 1);
    assert_eq!(inth_root(100, 1), 100);
    assert_eq!(inth_root(u128::MAX, 1), u128::MAX);
}

#[test]
fn test_inth_root_k2_delegates_to_isqrt() {
    assert_eq!(inth_root(0, 2), 0);
    assert_eq!(inth_root(9, 2), 3);
    assert_eq!(inth_root(100, 2), 10);
    assert_eq!(inth_root(1_000_000, 2), 1_000);
}

#[test]
fn test_inth_root_k3_perfect_cubes() {
    assert_eq!(inth_root(8, 3), 2);      // 2^3 = 8
    assert_eq!(inth_root(27, 3), 3);     // 3^3 = 27
    assert_eq!(inth_root(1000, 3), 10);  // 10^3 = 1000
    assert_eq!(inth_root(1_000_000_000, 3), 1000); // 1000^3
}

#[test]
fn test_inth_root_floor() {
    // floor(7^(1/2)) = 2
    assert_eq!(inth_root(7, 2), 2);
    // floor(26^(1/3)) = 2  (since 2^3=8, 3^3=27)
    assert_eq!(inth_root(26, 3), 2);
    // floor(28^(1/3)) = 3  (since 3^3=27, 4^3=64)
    assert_eq!(inth_root(28, 3), 3);
}

// ============================================================================
// leverage_multiplier tests
// ============================================================================

#[test]
fn test_leverage_mult_1x() {
    // notional == capital → leverage == 1.0 → multiplier == 1.0
    let (num, den) = leverage_multiplier(1_000_000, 1_000_000, 3, 2);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_leverage_mult_below_1x() {
    // notional < capital → clamped to 1.0
    let (num, den) = leverage_multiplier(500_000, 1_000_000, 3, 2);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_leverage_mult_zero_capital() {
    // capital == 0 → guard returns 1.0
    let (num, den) = leverage_multiplier(1_000_000, 0, 3, 2);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

/// Helper: ratio * 100 where ratio = (num / den) * scale
/// Returns (num * 100) / den
fn ratio_x100(num: u128, den: u128) -> u128 {
    (num * 100) / den
}

#[test]
fn test_leverage_mult_5x_exp_1_5() {
    // 5^1.5 ≈ 11.18 → ratio * 100 ≈ 1118 → accept 1060..1180
    let (num, den) = leverage_multiplier(
        5_000_000, // notional = 5x capital
        1_000_000,
        3, 2,
    );
    let r = ratio_x100(num, den);
    assert!(
        (1060..=1180).contains(&r),
        "5^1.5 * 100 should be in 1060..=1180, got {}",
        r
    );
}

#[test]
fn test_leverage_mult_10x_exp_1_5() {
    // 10^1.5 ≈ 31.62 → ratio * 100 ≈ 3162 → accept 3000..3320
    let (num, den) = leverage_multiplier(
        10_000_000,
        1_000_000,
        3, 2,
    );
    let r = ratio_x100(num, den);
    assert!(
        (3000..=3320).contains(&r),
        "10^1.5 * 100 should be in 3000..=3320, got {}",
        r
    );
}

#[test]
fn test_leverage_mult_25x_exp_1_5() {
    // 25^1.5 = 125.0 → ratio * 100 = 12500 → accept 11900..13100
    let (num, den) = leverage_multiplier(
        25_000_000,
        1_000_000,
        3, 2,
    );
    let r = ratio_x100(num, den);
    assert!(
        (11900..=13100).contains(&r),
        "25^1.5 * 100 should be in 11900..=13100, got {}",
        r
    );
}

#[test]
fn test_leverage_mult_100x_exp_1_5() {
    // 100^1.5 = 1000.0 → ratio * 100 = 100000 → accept 95000..105000
    let (num, den) = leverage_multiplier(
        100_000_000,
        1_000_000,
        3, 2,
    );
    let r = ratio_x100(num, den);
    assert!(
        (95000..=105000).contains(&r),
        "100^1.5 * 100 should be in 95000..=105000, got {}",
        r
    );
}

#[test]
fn test_leverage_mult_linear_exponent() {
    // exp (1,1) at 10x → multiplier ≈ 10.0 → ratio * 100 ≈ 1000
    let (num, den) = leverage_multiplier(
        10_000_000,
        1_000_000,
        1, 1,
    );
    let r = ratio_x100(num, den);
    // Allow generous tolerance: 900..1100
    assert!(
        (900..=1100).contains(&r),
        "10x linear multiplier * 100 should be ~1000, got {}",
        r
    );
}

#[test]
fn test_leverage_mult_quadratic_exponent() {
    // exp (2,1) at 10x → multiplier ≈ 100.0 → ratio * 100 ≈ 10000
    let (num, den) = leverage_multiplier(
        10_000_000,
        1_000_000,
        2, 1,
    );
    let r = ratio_x100(num, den);
    // Allow generous tolerance: 9000..11000
    assert!(
        (9000..=11000).contains(&r),
        "10x quadratic multiplier * 100 should be ~10000, got {}",
        r
    );
}

// ============================================================================
// leverage_tail_surcharge tests (Task 3)
// ============================================================================

// maintenance_margin_bps = 500 → L_max = 10_000 / 500 = 20x.
// threshold_bps = 8000 → surcharge onset at 80% of L_max = 16x.
// steepness = 3000 (3.0 in MULT_SCALE) → up to 1.0 + 3.0 = 4.0x at the boundary.

#[test]
fn test_tail_surcharge_below_threshold_neutral() {
    // 10x leverage, onset at 16x → below threshold → neutral 1.0x.
    let (num, den) = leverage_tail_surcharge(
        10_000_000, // notional
        1_000_000,  // capital → 10x
        500,        // maintenance_margin_bps → L_max = 20x
        8000,       // threshold_bps → onset at 16x
        3000,       // steepness
    );
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_tail_surcharge_at_boundary_max() {
    // 20x leverage = exactly L_max → full surcharge = 1.0 + steepness.
    let (num, den) = leverage_tail_surcharge(
        20_000_000, // notional
        1_000_000,  // capital → 20x = L_max
        500,
        8000,
        3000,
    );
    assert_eq!(den, MULT_SCALE);
    // 1.0 + 3.0 = 4.0x → 4000 in MULT_SCALE.
    assert_eq!(num, 4000);
}

#[test]
fn test_tail_surcharge_above_boundary_capped() {
    // Leverage beyond L_max (over-leveraged corrupt state) → capped at max.
    let (num, den) = leverage_tail_surcharge(
        50_000_000, // 50x >> L_max
        1_000_000,
        500,
        8000,
        3000,
    );
    assert_eq!(den, MULT_SCALE);
    assert_eq!(num, 4000);
}

#[test]
fn test_tail_surcharge_interpolates() {
    // 18x leverage. onset 16x, L_max 20x → position = (18-16)/(20-16) = 0.5.
    // surcharge = 1.0 + 0.5 * 3.0 = 2.5x → 2500.
    let (num, den) = leverage_tail_surcharge(
        18_000_000,
        1_000_000,
        500,
        8000,
        3000,
    );
    assert_eq!(den, MULT_SCALE);
    assert!(
        (2400..=2600).contains(&num),
        "tail surcharge at 18x (halfway) should be ~2500, got {}",
        num
    );
}

#[test]
fn test_tail_surcharge_zero_steepness_neutral() {
    // steepness 0 → disabled → always neutral even at the boundary.
    let (num, den) = leverage_tail_surcharge(20_000_000, 1_000_000, 500, 8000, 0);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_tail_surcharge_zero_maintenance_neutral() {
    // maintenance_margin_bps = 0 → no finite L_max → neutral (guard).
    let (num, den) = leverage_tail_surcharge(20_000_000, 1_000_000, 0, 8000, 3000);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

// ============================================================================
// calibrate_base_rate tests (Task 4)
// ============================================================================

#[test]
fn test_calibrate_base_rate_basic() {
    // target_loss_ratio = 1.0 (break-even): premium should recover claims.
    // base_rate = target_loss_ratio * cumulative_claims * PREMIUM_SCALE / exposure
    // With exposure = notional*slots = 1_000_000 * 1000 = 1e9, claims = 1000,
    // target = 1.0 (num=1,den=1):
    //   base_rate = 1 * 1000 * 1e9 / 1e9 = 1000
    let br = calibrate_base_rate(1, 1, 1_000, 1_000_000_000);
    assert_eq!(br, 1_000);
}

#[test]
fn test_calibrate_base_rate_loaded() {
    // target_loss_ratio = 0.5 (premiums = 2x claims, i.e. 50% loss ratio):
    // base_rate = 0.5_inverse? No — loss_ratio = claims/premium. To hit a target
    // loss ratio L, premium = claims / L, so base_rate scales by 1/L.
    // target L = 1/2 → base_rate doubles vs break-even.
    let br_breakeven = calibrate_base_rate(1, 1, 1_000, 1_000_000_000);
    let br_loaded = calibrate_base_rate(1, 2, 1_000, 1_000_000_000);
    assert_eq!(br_loaded, 2 * br_breakeven);
}

#[test]
fn test_calibrate_base_rate_zero_exposure() {
    // No observed exposure → cannot calibrate → returns 0.
    assert_eq!(calibrate_base_rate(1, 1, 1_000, 0), 0);
}

#[test]
fn test_calibrate_base_rate_zero_claims() {
    // No observed claims → base_rate 0 (nothing to recover).
    assert_eq!(calibrate_base_rate(1, 1, 0, 1_000_000_000), 0);
}

#[test]
fn test_calibrate_base_rate_uses_premium_scale() {
    // Verify PREMIUM_SCALE is the scaling unit so the result plugs straight
    // into compute_premium_per_slot's base_rate slot.
    // exposure = 2e9, claims = 4000, target 1.0:
    //   base_rate = 4000 * PREMIUM_SCALE / 2e9 = 4000 * 1e9 / 2e9 = 2000
    let br = calibrate_base_rate(1, 1, 4_000, 2_000_000_000);
    assert_eq!(br, 4_000 * PREMIUM_SCALE / 2_000_000_000);
}

// ============================================================================
// compute_premium_per_slot tests
// ============================================================================

#[test]
fn test_premium_zero_notional() {
    let idx = RiskIndex::neutral();
    let result = compute_premium_per_slot(0, 1_000_000, 100, &idx, 0);
    assert_eq!(result, 0, "zero notional should return 0");
}

#[test]
fn test_premium_basic_calculation() {
    // notional=60_000, capital=40_000 → 1.5x leverage, neutral multipliers
    let idx = RiskIndex::neutral();
    let result = compute_premium_per_slot(60_000, 40_000, 100, &idx, 0);
    assert!(result > 0, "basic premium should be positive, got {}", result);
}

#[test]
fn test_premium_increases_with_leverage() {
    let idx = RiskIndex::neutral();
    // 5x leverage: notional=50_000, capital=10_000
    let prem_5x = compute_premium_per_slot(50_000, 10_000, 100, &idx, 0);
    // 25x leverage: notional=250_000, capital=10_000
    let prem_25x = compute_premium_per_slot(250_000, 10_000, 100, &idx, 0);
    // 100x leverage: notional=1_000_000, capital=10_000
    let prem_100x = compute_premium_per_slot(1_000_000, 10_000, 100, &idx, 0);

    assert!(prem_5x > 0, "5x premium should be positive, got {}", prem_5x);
    assert!(
        prem_25x > prem_5x,
        "25x premium ({}) should exceed 5x premium ({})",
        prem_25x, prem_5x
    );
    assert!(
        prem_100x > prem_25x,
        "100x premium ({}) should exceed 25x premium ({})",
        prem_100x, prem_25x
    );
    // Superlinear: 100x vs 5x ratio should be > 50
    let ratio = prem_100x / prem_5x;
    assert!(
        ratio > 50,
        "100x/5x premium ratio should be > 50 (superlinear), got {}",
        ratio
    );
}

#[test]
fn test_premium_increases_with_crowding() {
    // Neutral index
    let neutral_idx = RiskIndex::neutral();
    // Crowded index: crowding multiplier = 3x
    let crowded_idx = RiskIndex {
        crowding: (3 * MULT_SCALE, MULT_SCALE),
        oi_vault: (MULT_SCALE, MULT_SCALE),
        pool_health: (MULT_SCALE, MULT_SCALE),
        volatility: (MULT_SCALE, MULT_SCALE),
        leverage_tail: (MULT_SCALE, MULT_SCALE),
    };

    // 10x leverage position — use large notional/base_rate for integer resolution
    let prem_normal = compute_premium_per_slot(100_000_000, 10_000_000, 100_000, &neutral_idx, 0);
    let prem_crowded = compute_premium_per_slot(100_000_000, 10_000_000, 100_000, &crowded_idx, 0);

    assert!(
        prem_crowded > prem_normal,
        "crowded premium ({}) should exceed normal premium ({})",
        prem_crowded, prem_normal
    );

    // Ratio should be ~3x (accept 250..350 for ratio*100)
    let ratio_x100 = (prem_crowded * 100) / prem_normal;
    assert!(
        (250..=350).contains(&ratio_x100),
        "crowded/normal ratio * 100 should be ~300 (3x), got {}",
        ratio_x100
    );
}

#[test]
fn test_premium_increases_with_volatility() {
    // Task 2: the covered loss is gap risk, so premium must scale with realized
    // volatility exactly like the other multipliers.
    let neutral_idx = RiskIndex::neutral();
    // Volatility multiplier = 2x, all else neutral.
    let vol_idx = RiskIndex {
        crowding: (MULT_SCALE, MULT_SCALE),
        oi_vault: (MULT_SCALE, MULT_SCALE),
        pool_health: (MULT_SCALE, MULT_SCALE),
        volatility: (2 * MULT_SCALE, MULT_SCALE),
        leverage_tail: (MULT_SCALE, MULT_SCALE),
    };

    let prem_normal = compute_premium_per_slot(100_000_000, 10_000_000, 100_000, &neutral_idx, 0);
    let prem_vol = compute_premium_per_slot(100_000_000, 10_000_000, 100_000, &vol_idx, 0);

    assert!(
        prem_vol > prem_normal,
        "volatile premium ({}) should exceed normal premium ({})",
        prem_vol, prem_normal
    );
    // Ratio should be ~2x (accept 180..220 for ratio*100).
    let ratio_x100 = (prem_vol * 100) / prem_normal;
    assert!(
        (180..=220).contains(&ratio_x100),
        "volatile/normal ratio * 100 should be ~200 (2x), got {}",
        ratio_x100
    );
}

#[test]
fn test_premium_min_floor() {
    // Very tiny notional, high capital → near-zero computed premium
    let idx = RiskIndex::neutral();
    let result = compute_premium_per_slot(1, 1_000_000, 100, &idx, 100);
    assert!(
        result >= 100,
        "min_premium floor should apply, got {} (expected >= 100)",
        result
    );
}
