use percolator_insurance::risk_index::{RiskIndex, crowding_multiplier, oi_vault_multiplier, pool_health_multiplier};
use percolator_insurance::MULT_SCALE;

// ============================================================================
// RiskIndex struct tests
// ============================================================================

#[test]
fn test_risk_index_neutral() {
    let ri = RiskIndex::neutral();
    assert_eq!(ri.crowding, (MULT_SCALE, MULT_SCALE));
    assert_eq!(ri.oi_vault, (MULT_SCALE, MULT_SCALE));
    assert_eq!(ri.pool_health, (MULT_SCALE, MULT_SCALE));
}

// ============================================================================
// crowding_multiplier tests
// ============================================================================

#[test]
fn test_crowding_balanced() {
    // Equal OI on both sides — even the majority side gets 1.0 (ratio = 1.0 <= 1.5 threshold)
    let (num, den) = crowding_multiplier(
        1_000_000, // oi_majority
        1_000_000, // oi_minority
        true,      // is_majority_side
        1500, 1000, // low_ratio = 1.5
        5000, 1000, // high_ratio = 5.0
        4000,      // cap = 4.0
    );
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_crowding_minority_side() {
    // Minority side always gets 1.0 regardless of imbalance
    let (num, den) = crowding_multiplier(
        9_000_000, // oi_majority — 9x imbalance
        1_000_000, // oi_minority
        false,     // is_majority_side = false → minority side
        1500, 1000,
        5000, 1000,
        4000,
    );
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_crowding_majority_max() {
    // ratio >> cap threshold → capped at 4000
    let (num, den) = crowding_multiplier(
        100_000_000, // oi_majority
        1_000_000,   // oi_minority — ratio = 100x >> 5.0 threshold
        true,
        1500, 1000,
        5000, 1000,
        4000,
    );
    assert_eq!(num, 4000);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_crowding_one_side_empty() {
    // minority = 0 → return cap
    let (num, den) = crowding_multiplier(
        5_000_000,
        0, // minority = 0
        true,
        1500, 1000,
        5000, 1000,
        4000,
    );
    assert_eq!(num, 4000);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_crowding_interpolation() {
    // ratio = 3.0, range [1.5, 5.0], cap = 4.0x (4000)
    // position in range = (3.0 - 1.5) / (5.0 - 1.5) = 1.5 / 3.5 ≈ 0.4286
    // multiplier = 1.0 + 0.4286 * (4.0 - 1.0) ≈ 1.0 + 1.286 ≈ 2.286
    // In MULT_SCALE units: ~2286
    // Accept 2100..2500 range per spec
    let (num, den) = crowding_multiplier(
        3_000_000, // oi_majority
        1_000_000, // oi_minority — ratio = 3.0
        true,
        1500, 1000, // low = 1.5
        5000, 1000, // high = 5.0
        4000,       // cap = 4.0
    );
    assert_eq!(den, MULT_SCALE);
    assert!(
        (2100..=2500).contains(&num),
        "crowding interp at ratio=3.0 should be ~2286, got {}",
        num
    );
}

// ============================================================================
// oi_vault_multiplier tests
// ============================================================================

#[test]
fn test_oi_vault_low_leverage() {
    // OI 100M, vault 500M → sys_lev = 100/500 = 0.2 < floor=1.0 → 1.0
    let (num, den) = oi_vault_multiplier(
        100_000_000, // total_oi_notional
        500_000_000, // vault
        1000, 1000,  // floor_ratio = 1.0
        5000, 1000,  // cap_ratio = 5.0
        3000,        // mult_max = 3.0
    );
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_oi_vault_high_leverage() {
    // OI 3B, vault 500M → sys_lev = 6.0 > cap=5.0 → max (3000)
    let (num, den) = oi_vault_multiplier(
        3_000_000_000, // total_oi_notional
        500_000_000,   // vault
        1000, 1000,    // floor_ratio = 1.0
        5000, 1000,    // cap_ratio = 5.0
        3000,          // mult_max = 3.0
    );
    assert_eq!(num, 3000);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_oi_vault_zero_vault() {
    // vault = 0 → max multiplier
    let (num, den) = oi_vault_multiplier(
        1_000_000,
        0,          // vault = 0
        1000, 1000,
        5000, 1000,
        3000,
    );
    assert_eq!(num, 3000);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_oi_vault_interpolation() {
    // sys_lev = 3.0, range [1.0, 5.0], max = 3.0x (3000)
    // position = (3.0 - 1.0) / (5.0 - 1.0) = 2.0 / 4.0 = 0.5
    // multiplier = 1.0 + 0.5 * (3.0 - 1.0) = 1.0 + 1.0 = 2.0x (2000)
    // Accept 1800..2200
    let (num, den) = oi_vault_multiplier(
        1_500_000_000, // total_oi_notional = 3x vault
        500_000_000,   // vault
        1000, 1000,    // floor_ratio = 1.0
        5000, 1000,    // cap_ratio = 5.0
        3000,          // mult_max = 3.0
    );
    assert_eq!(den, MULT_SCALE);
    assert!(
        (1800..=2200).contains(&num),
        "oi_vault interp at sys_lev=3.0 should be ~2000, got {}",
        num
    );
}

// ============================================================================
// pool_health_multiplier tests
// ============================================================================

#[test]
fn test_pool_health_healthy() {
    // pool 50M, OI 600M → health = 50/600 ≈ 8.33% > 5% high_health → 1.0
    let (num, den) = pool_health_multiplier(
        50_000_000,  // pool_balance
        600_000_000, // total_oi_notional
        1, 100,      // low_health = 1%
        5, 100,      // high_health = 5%
        5000,        // mult_max = 5.0
    );
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_pool_health_depleted() {
    // pool 2M, OI 600M → health = 2/600 ≈ 0.33% < 1% low_health → max (5000)
    let (num, den) = pool_health_multiplier(
        2_000_000,   // pool_balance
        600_000_000, // total_oi_notional
        1, 100,      // low_health = 1%
        5, 100,      // high_health = 5%
        5000,        // mult_max = 5.0
    );
    assert_eq!(num, 5000);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_pool_health_zero_oi() {
    // total_oi_notional = 0 → no risk → 1.0
    let (num, den) = pool_health_multiplier(
        50_000_000,
        0, // zero OI
        1, 100,
        5, 100,
        5000,
    );
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_pool_health_interpolation() {
    // pool 18M, OI 600M → health = 18/600 = 3%
    // range [1%, 5%], max = 5.0x (5000)
    // position from high end: (5% - 3%) / (5% - 1%) = 2 / 4 = 0.5
    // multiplier = 1.0 + 0.5 * (5.0 - 1.0) = 3.0x (3000)
    // Accept 2700..3300
    let (num, den) = pool_health_multiplier(
        18_000_000,  // pool_balance
        600_000_000, // total_oi_notional
        1, 100,      // low_health = 1%
        5, 100,      // high_health = 5%
        5000,        // mult_max = 5.0
    );
    assert_eq!(den, MULT_SCALE);
    assert!(
        (2700..=3300).contains(&num),
        "pool_health interp at 3% should be ~3000, got {}",
        num
    );
}
