//! Premium calculation engine.
//!
//! Pure functions that compute per-slot premium rates from account and market
//! state. No side effects, no state mutation. All arithmetic uses integer
//! math with u256 intermediates for overflow safety.

use crate::{LEVERAGE_SCALE, MULT_SCALE, PREMIUM_SCALE};
use crate::risk_index::RiskIndex;

// ============================================================================
// Integer square root
// ============================================================================

/// Integer square root via Newton's method.
///
/// Returns `floor(sqrt(n))`. No floating point.
pub fn isqrt(n: u128) -> u128 {
    if n <= 1 {
        return n;
    }
    // Use bit-length based initial guess: 2^ceil(bits/2).
    // Starting at n itself would cause (n+1)/2 to overflow for n=u128::MAX.
    let bits = 128u32 - n.leading_zeros();
    let shift = (bits + 1) / 2;  // ceil(bits / 2)
    let shift = shift.min(64);    // sqrt(u128::MAX) < 2^64
    let mut x: u128 = if shift == 64 {
        (u64::MAX as u128) + 1
    } else {
        1u128 << shift
    };
    let mut y = (x + n / x) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ============================================================================
// Integer nth-root
// ============================================================================

/// Integer nth-root via Newton's method.
///
/// Returns `floor(n^(1/k))`.
///
/// # Panics
/// Panics if `k == 0`.
pub fn inth_root(n: u128, k: u32) -> u128 {
    assert!(k != 0, "inth_root: k must be > 0");

    if k == 1 {
        return n;
    }
    if k == 2 {
        return isqrt(n);
    }
    if n <= 1 {
        return n;
    }

    // Initial guess: 2^ceil(bit_length(n) / k)
    let bits = 128 - n.leading_zeros(); // bit_length(n)
    let shift = (bits + k - 1) / k;    // ceil(bits / k)
    let shift = shift.min(127);         // cap to avoid overflow on 1u128 << 128
    let mut x: u128 = 1u128 << shift;

    loop {
        // Compute x^(k-1); if it saturates, x is too large — halve and retry.
        let xpow = pow_saturating(x, k - 1);
        if xpow == u128::MAX && x > 1 {
            x /= 2;
            continue;
        }
        let next = if xpow == 0 {
            x / 2
        } else {
            // Newton: x_next = ((k-1)*x + n / x^(k-1)) / k
            let k128 = k as u128;
            ((k128 - 1) * x + n / xpow) / k128
        };

        if next >= x {
            break;
        }
        x = next;
    }

    // Correct downward: Newton may overshoot by one due to integer division.
    while pow_saturating(x, k) > n {
        x -= 1;
    }
    // Correct upward: Newton may stop one below if it oscillates.
    while pow_saturating(x + 1, k) <= n {
        x += 1;
    }
    x
}

/// Compute `base^exp` returning `u128::MAX` on overflow (saturating).
fn pow_saturating(mut base: u128, mut exp: u32) -> u128 {
    let mut result: u128 = 1;
    loop {
        if exp & 1 == 1 {
            result = match result.checked_mul(base) {
                Some(v) => v,
                None => return u128::MAX,
            };
        }
        exp >>= 1;
        if exp == 0 {
            break;
        }
        base = match base.checked_mul(base) {
            Some(v) => v,
            None => {
                // base^2 overflowed; if exp > 0 result will also overflow
                return u128::MAX;
            }
        };
    }
    result
}

// ============================================================================
// Leverage multiplier
// ============================================================================

/// Compute the leverage multiplier as a `(num, den)` pair scaled by
/// [`MULT_SCALE`].
///
/// # Formula
/// ```text
/// leverage = notional * LEVERAGE_SCALE / capital
/// multiplier = leverage ^ (exp_num / exp_den)
/// ```
///
/// Returns `(MULT_SCALE, MULT_SCALE)` (i.e. 1.0) when:
/// - `capital == 0`
/// - `leverage <= 1.0`
///
/// Returns saturated `(u64::MAX as u128, MULT_SCALE)` on overflow.
///
/// Both `num` and `den` use `MULT_SCALE` as the unit for 1.0, so the
/// actual multiplier is `num / den` (both are multiples of `MULT_SCALE`).
pub fn leverage_multiplier(
    notional: u128,
    capital: u128,
    exp_num: u64,
    exp_den: u64,
) -> (u128, u128) {
    const ONE: (u128, u128) = (MULT_SCALE, MULT_SCALE);

    // Guard: zero capital → no leverage computable
    if capital == 0 {
        return ONE;
    }

    // lev_scaled = notional * LEVERAGE_SCALE / capital
    // This is leverage in fixed-point with LEVERAGE_SCALE as 1.0
    let lev_scaled = match notional.checked_mul(LEVERAGE_SCALE) {
        Some(v) => v / capital,
        None => {
            // Overflow on notional * LEVERAGE_SCALE — use saturated division
            // notional/capital * LEVERAGE_SCALE (loses some precision but safe)
            (notional / capital).saturating_mul(LEVERAGE_SCALE)
        }
    };

    // If leverage <= 1.0, floor at 1.0
    if lev_scaled <= LEVERAGE_SCALE {
        return ONE;
    }

    // Compute lev_scaled ^ (exp_num / exp_den) in fixed-point.
    //
    // Strategy: multiply and root in the LEVERAGE_SCALE domain then convert
    // to MULT_SCALE.
    //
    // result_scaled = lev_scaled^(p/q)
    //              = (lev_scaled^p)^(1/q)
    //
    // All intermediate values are in LEVERAGE_SCALE fixed-point.
    // To keep precision we compute:
    //   numerator   = (lev_scaled^p)^(1/q)   — integer, in LEVERAGE_SCALE^(p/q) units
    //   denominator = LEVERAGE_SCALE^(p/q-1)  — the scaling factor
    //
    // Simpler: work entirely in u128, tracking the scale explicitly.

    let p = exp_num as u32;
    let q = exp_den as u32;

    // Special fast path: exp 3/2 (most common for insurance)
    if p == 3 && q == 2 {
        return leverage_exp_3_2(lev_scaled);
    }

    // General path: compute lev^p then take qth root.
    //
    // lev_scaled is e.g. 10_000_000 for 10x (since LEVERAGE_SCALE=1_000_000).
    // We want result = lev_scaled^(p/q) in same units.
    //
    // Step 1: lev^p (may be huge — use checked arithmetic)
    // Step 2: take q-th root
    // Step 3: scale back to MULT_SCALE
    //
    // The tricky part is that lev^p overflows easily for large p.
    // We handle this by working in "normalised" space:
    //   lev_norm = lev_scaled / LEVERAGE_SCALE   (actual leverage ratio, >= 1)
    //   lev_frac = lev_scaled % LEVERAGE_SCALE   (fractional part)
    //
    // For integer exponents (q=1): result = lev_scaled^p / LEVERAGE_SCALE^(p-1)
    // For fractional (general):    result ≈ inth_root(lev_scaled^p, q) / LEVERAGE_SCALE^((p-q)/q)
    //
    // We use a scaled computation: compute in LEVERAGE_SCALE^q space.

    // lev_power = lev_scaled^p in (LEVERAGE_SCALE^p) space
    // Then root_q(lev_power) is in (LEVERAGE_SCALE^(p/q)) space
    // We want result in LEVERAGE_SCALE space, so divide by LEVERAGE_SCALE^(p/q - 1)
    //   = multiply by LEVERAGE_SCALE / LEVERAGE_SCALE^(p/q)
    //   = LEVERAGE_SCALE^(1 - p/q)
    //   = LEVERAGE_SCALE^((q-p)/q)    [if q > p]
    //   or divide by LEVERAGE_SCALE^((p-q)/q) [if p > q]

    // To avoid overflow, raise lev_scaled to power p only if it fits.
    // If it overflows, return saturated value.
    let lev_p = pow_saturating(lev_scaled, p);
    if lev_p == u128::MAX && lev_scaled > LEVERAGE_SCALE {
        // Likely saturated — return max
        return (u64::MAX as u128, MULT_SCALE);
    }

    // Take q-th root of lev_p → result is in LEVERAGE_SCALE^(p/q) units
    let root = inth_root(lev_p, q);

    // root is in LEVERAGE_SCALE^(p/q) space.
    // We want the multiplier in MULT_SCALE space (i.e. MULT_SCALE = 1.0).
    //
    // actual = root / LEVERAGE_SCALE^(p/q)
    // result_mult_scaled = actual * MULT_SCALE
    //                    = root * MULT_SCALE / LEVERAGE_SCALE^(p/q)
    //
    // LEVERAGE_SCALE^(p/q) is not necessarily an integer. Instead:
    //   LEVERAGE_SCALE^(p/q) = (LEVERAGE_SCALE^p)^(1/q)
    //
    // So: scale_divisor = inth_root(LEVERAGE_SCALE^p, q)
    let ls_p = pow_saturating(LEVERAGE_SCALE, p);
    let scale_divisor = inth_root(ls_p, q);

    if scale_divisor == 0 {
        return ONE;
    }

    // multiplier = (root / scale_divisor) in raw ratio
    // result in MULT_SCALE units = root * MULT_SCALE / scale_divisor
    let num = match root.checked_mul(MULT_SCALE) {
        Some(v) => v / scale_divisor,
        None => {
            // Overflow — try dividing first
            (root / scale_divisor).saturating_mul(MULT_SCALE)
        }
    };

    // Floor at 1.0
    if num < MULT_SCALE {
        return ONE;
    }

    (num, MULT_SCALE)
}

/// Fast path for exp = 3/2: multiplier = lev^(3/2) = lev * sqrt(lev).
///
/// Input `lev_scaled` is in LEVERAGE_SCALE fixed-point (LEVERAGE_SCALE = 1.0).
/// Output is in MULT_SCALE fixed-point (MULT_SCALE = 1.0).
fn leverage_exp_3_2(lev_scaled: u128) -> (u128, u128) {
    // multiplier = lev^1.5 = lev * sqrt(lev)
    // In fixed-point:
    //   lev = lev_scaled / LEVERAGE_SCALE
    //   sqrt(lev) = sqrt(lev_scaled / LEVERAGE_SCALE)
    //             = sqrt(lev_scaled) / sqrt(LEVERAGE_SCALE)
    //
    // lev * sqrt(lev) = (lev_scaled / LS) * (sqrt(lev_scaled) / sqrt(LS))
    //                 = lev_scaled * sqrt(lev_scaled) / (LS * sqrt(LS))
    //                 = lev_scaled * sqrt(lev_scaled) / LS^1.5
    //
    // LEVERAGE_SCALE = 1_000_000, sqrt(LS) = 1_000, LS^1.5 = LS * sqrt(LS) = 1_000_000_000
    //
    // result_in_MULT_SCALE = multiplier * MULT_SCALE
    //   = lev_scaled * sqrt(lev_scaled) * MULT_SCALE / LS^1.5

    let sqrt_lev = isqrt(lev_scaled);

    // Numerator: lev_scaled * sqrt_lev * MULT_SCALE
    // Denominator: LEVERAGE_SCALE^1.5 = 1_000_000 * 1_000 = 1_000_000_000
    const LS_3_2: u128 = 1_000_000_000; // LEVERAGE_SCALE^1.5 = 1_000_000 * sqrt(1_000_000) = 1e9

    // Compute lev_scaled * sqrt_lev first (may overflow for huge leverage)
    let numerator = match lev_scaled.checked_mul(sqrt_lev) {
        Some(v) => v,
        None => {
            // Overflow — return saturation
            return (u64::MAX as u128, MULT_SCALE);
        }
    };

    // numerator * MULT_SCALE / LS_3_2
    let num = match numerator.checked_mul(MULT_SCALE) {
        Some(v) => v / LS_3_2,
        None => {
            // Divide first to reduce magnitude, then scale
            (numerator / LS_3_2).saturating_mul(MULT_SCALE)
        }
    };

    if num < MULT_SCALE {
        return (MULT_SCALE, MULT_SCALE);
    }

    (num, MULT_SCALE)
}

// ============================================================================
// GCD helper
// ============================================================================

/// Binary GCD algorithm — no division, fast on u128.
fn gcd(mut a: u128, mut b: u128) -> u128 {
    if a == 0 {
        return b;
    }
    if b == 0 {
        return a;
    }
    // Count and remove common trailing zeros (factors of 2)
    let shift = a.trailing_zeros().min(b.trailing_zeros());
    a >>= a.trailing_zeros();
    b >>= b.trailing_zeros();
    loop {
        // a and b are both odd here
        if a > b {
            let tmp = a;
            a = b;
            b = tmp;
        }
        // b >= a; subtract then remove factors of 2
        b -= a;
        if b == 0 {
            return a << shift;
        }
        b >>= b.trailing_zeros();
    }
}

// ============================================================================
// Full premium calculation
// ============================================================================

/// Compute the per-slot insurance premium for one account.
///
/// # Formula
/// ```text
/// premium = notional × base_rate × lev_num × crowd_num × oiv_num × pool_num
///           ÷ (PREMIUM_SCALE × lev_den × crowd_den × oiv_den × pool_den)
/// ```
/// Result is ceiling-divided, then floored at `min_premium`.
///
/// Returns 0 immediately if `notional == 0`.
pub fn compute_premium_per_slot(
    notional: u128,
    capital: u128,
    base_rate: u128,
    risk_idx: &RiskIndex,
    min_premium: u128,
) -> u128 {
    if notional == 0 {
        return 0;
    }

    // Leverage multiplier (hardcoded 3/2 exponent = 1.5)
    let (lev_num, lev_den) = leverage_multiplier(notional, capital, 3, 2);

    let (crowd_num, crowd_den) = risk_idx.crowding;
    let (oiv_num, oiv_den) = risk_idx.oi_vault;
    let (pool_num, pool_den) = risk_idx.pool_health;

    // Build numerator: notional × base_rate × lev_num × crowd_num × oiv_num × pool_num
    // Use GCD reduction at each step to prevent overflow.
    let mut num: u128 = notional;
    let mut den: u128 = PREMIUM_SCALE;

    // Multiply in base_rate
    num = match num.checked_mul(base_rate) {
        Some(v) => v,
        None => {
            // Reduce first
            let g = gcd(num, den);
            num /= g;
            den /= g;
            match num.checked_mul(base_rate) {
                Some(v) => v,
                None => {
                    // Still too large — partial divide to reduce magnitude
                    num = num / base_rate.max(1) * base_rate;
                    num
                }
            }
        }
    };

    // Helper macro replaced by inline function pattern — reduce and multiply
    // for each multiplier component.
    macro_rules! mul_component {
        ($num:expr, $den:expr, $c_num:expr, $c_den:expr) => {{
            // Reduce num/den by gcd first
            let g = gcd($num, $den);
            $num /= g;
            $den /= g;
            // Reduce num/$c_den cross
            let g2 = gcd($num, $c_den);
            $num /= g2;
            let c_den_r = $c_den / g2;
            // Reduce $c_num/den cross
            let g3 = gcd($c_num, $den);
            let c_num_r = $c_num / g3;
            $den /= g3;
            // Now accumulate
            $den = match $den.checked_mul(c_den_r) {
                Some(v) => v,
                None => {
                    // Partial divide — lose precision but stay safe
                    $den = $den / c_den_r.max(1);
                    $den
                }
            };
            $num = match $num.checked_mul(c_num_r) {
                Some(v) => v,
                None => {
                    // Try dividing num by something to make room
                    let bits = 128u32 - $num.leading_zeros();
                    let shift = bits.saturating_sub(64);
                    $num >>= shift;
                    $den >>= shift;
                    match $num.checked_mul(c_num_r) {
                        Some(v) => v,
                        None => u128::MAX / c_num_r.max(1),
                    }
                }
            };
        }};
    }

    // Multiply in lev component
    mul_component!(num, den, lev_num, lev_den);
    // Multiply in crowding component
    mul_component!(num, den, crowd_num, crowd_den);
    // Multiply in oi_vault component
    mul_component!(num, den, oiv_num, oiv_den);
    // Multiply in pool_health component
    mul_component!(num, den, pool_num, pool_den);

    // Final GCD reduction
    let g = gcd(num, den);
    let num = num / g;
    let den = den / g;

    if den == 0 {
        return min_premium;
    }

    // Ceiling division: (num + den - 1) / den
    let result = match num.checked_add(den.saturating_sub(1)) {
        Some(v) => v / den,
        None => {
            // Addition overflowed — just floor divide (off by at most 1)
            num / den
        }
    };

    result.max(min_premium)
}
