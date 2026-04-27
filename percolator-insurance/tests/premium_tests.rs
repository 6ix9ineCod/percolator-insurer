use percolator_insurance::premium::{isqrt, inth_root, leverage_multiplier};
use percolator_insurance::MULT_SCALE;

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
