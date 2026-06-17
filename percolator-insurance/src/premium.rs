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
    let shift = bits.div_ceil(2);
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
    let shift = bits.div_ceil(k);
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

    if capital == 0 || notional == 0 || exp_den == 0 {
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
            core::mem::swap(&mut a, &mut b);
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
// System index (global accumulator)
// ============================================================================

/// Collapse the three account-independent system multipliers
/// (`oi_vault × pool_health × volatility`) into a single scalar in `MULT_SCALE`
/// units (`MULT_SCALE` == 1.0). Used by the global accrual accumulator.
///
/// `S = (oiv_num·pool_num·vol_num·MULT_SCALE) / (oiv_den·pool_den·vol_den)`,
/// GCD-reduced step by step, saturating to `u128::MAX` on true overflow
/// (conservative — consistent with the premium overflow policy).
pub fn compute_system_index_scaled(
    oi_vault: (u128, u128),
    pool_health: (u128, u128),
    volatility: (u128, u128),
) -> u128 {
    let mut num: u128 = MULT_SCALE;
    let mut den: u128 = 1;
    for (c_num, c_den) in [oi_vault, pool_health, volatility] {
        if c_den == 0 {
            continue;
        }
        let g1 = gcd(num, c_den);
        num /= g1;
        let c_den_r = c_den / g1;
        let g2 = gcd(c_num, den);
        let c_num_r = c_num / g2;
        den /= g2;
        num = num.saturating_mul(c_num_r);
        den = den.saturating_mul(c_den_r);
    }
    if den == 0 {
        return u128::MAX;
    }
    let g = gcd(num, den);
    (num / g) / (den / g)
}

// ============================================================================
// Leverage tail-surcharge (Task 3)
// ============================================================================

/// Pure-integer leverage tail surcharge as a `(num, den)` pair scaled by
/// [`MULT_SCALE`].
///
/// # Why
/// The base leverage factor (`leverage^1.5`) under-prices the tail as leverage
/// approaches the maintenance limit. The maximum permissible leverage is
/// `L_max = 10_000 / maintenance_margin_bps` (the reciprocal of the maintenance
/// margin). As leverage approaches `L_max`, the liquidation buffer between the
/// position's equity and the bankruptcy point collapses, so the probability the
/// engine cannot close the position without a socialized deficit rises sharply.
/// This surcharge steepens the leverage multiplier in exactly that region.
///
/// # Curve
/// Let `L` = current leverage, `L_max = 10_000 / maintenance_margin_bps`, and
/// `L_on = L_max * threshold_bps / 10_000` the onset leverage. Then:
/// - `L <= L_on`            → 1.0x (neutral)
/// - `L_on < L < L_max`     → linear ramp `1.0 + position * steepness`
///   where `position = (L - L_on) / (L_max - L_on)`
/// - `L >= L_max`           → `1.0 + steepness` (capped)
///
/// `steepness` is in `MULT_SCALE` units (e.g. 3000 = +3.0x at the boundary,
/// giving a 4.0x surcharge). The result is multiplied ON TOP OF the existing
/// `leverage^1.5` factor, so the effective leverage exponent rises near the
/// boundary without rewriting the base curve.
///
/// Returns neutral `(MULT_SCALE, MULT_SCALE)` when disabled or undefined:
/// `capital == 0`, `notional == 0`, `maintenance_margin_bps == 0`,
/// `steepness == 0`, or `threshold_bps >= 10_000` (degenerate onset == L_max).
///
/// CALIBRATION REQUIRED: the onset (`threshold_bps`) and slope (`steepness`)
/// are governance defaults. The exact curve MUST be fit to an empirical loss
/// distribution near the maintenance boundary — a linear ramp is a deliberately
/// simple, conservative placeholder, not a calibrated tail model.
pub fn leverage_tail_surcharge(
    notional: u128,
    capital: u128,
    maintenance_margin_bps: u128,
    threshold_bps: u128,
    steepness: u128,
) -> (u128, u128) {
    const ONE: (u128, u128) = (MULT_SCALE, MULT_SCALE);

    if capital == 0
        || notional == 0
        || maintenance_margin_bps == 0
        || steepness == 0
        || threshold_bps >= 10_000
    {
        return ONE;
    }

    // Work in LEVERAGE_SCALE fixed-point for L, L_on, L_max.
    //   lev_scaled = notional * LEVERAGE_SCALE / capital   (1.0x = LEVERAGE_SCALE)
    let lev_scaled = match notional.checked_mul(LEVERAGE_SCALE) {
        Some(v) => v / capital,
        None => (notional / capital).saturating_mul(LEVERAGE_SCALE),
    };

    // L_max = 10_000 / maintenance_margin_bps, in LEVERAGE_SCALE units:
    //   lmax_scaled = 10_000 * LEVERAGE_SCALE / maintenance_margin_bps
    let lmax_scaled = LEVERAGE_SCALE
        .saturating_mul(10_000)
        / maintenance_margin_bps;

    // Onset L_on = L_max * threshold_bps / 10_000, in LEVERAGE_SCALE units.
    let lon_scaled = lmax_scaled.saturating_mul(threshold_bps) / 10_000;

    // Below onset → neutral.
    if lev_scaled <= lon_scaled {
        return ONE;
    }

    let max_num = MULT_SCALE.saturating_add(steepness);

    // At or beyond L_max → full surcharge (cap).
    if lev_scaled >= lmax_scaled {
        return (max_num, MULT_SCALE);
    }

    // Linear interpolation between onset (1.0x) and L_max (1.0 + steepness).
    //   position = (lev - L_on) / (L_max - L_on)
    //   num = MULT_SCALE + position * steepness
    let range = lmax_scaled.saturating_sub(lon_scaled);
    if range == 0 {
        return (max_num, MULT_SCALE);
    }
    let delta = lev_scaled.saturating_sub(lon_scaled);
    let bump = delta.saturating_mul(steepness) / range;
    let num = MULT_SCALE.saturating_add(bump);

    (num, MULT_SCALE)
}

// ============================================================================
// base_rate calibration helper (Task 4)
// ============================================================================

/// Derive a `base_rate_per_slot` from observed loss experience and a target
/// loss ratio.
///
/// # Background
/// `base_rate` is otherwise an **uncalibrated free parameter** — nothing in this
/// crate fixes its level; it is a governance dial. This helper is how a
/// deployment would set it from real data: given a target loss ratio and the
/// realized claims/exposure, it solves for the per-slot base rate that, applied
/// across the observed exposure, would have produced premiums hitting that
/// target loss ratio.
///
/// # Model
/// Loss ratio `L = cumulative_claims / total_premium`. We want premium such
/// that `L = target`, i.e. `total_premium = cumulative_claims / target`.
/// Premium scales linearly with `base_rate` and with exposure
/// (`notional · slots`), so with all multipliers neutral:
///   `total_premium ≈ base_rate · exposure / PREMIUM_SCALE`
/// Solving for `base_rate`:
///   `base_rate = cumulative_claims · PREMIUM_SCALE / (target · exposure)`
/// With `target = target_loss_ratio_num / target_loss_ratio_den`:
///   `base_rate = cumulative_claims · PREMIUM_SCALE · target_den
///                / (target_num · exposure)`
///
/// A lower target loss ratio (more conservative / more loaded) yields a higher
/// base rate. `target_num = target_den` is break-even (loss ratio 1.0).
///
/// Returns 0 when uncalibratable: `observed_exposure == 0`,
/// `cumulative_claims == 0`, or `target_loss_ratio_num == 0`.
///
/// `observed_exposure` is cumulative `notional · slots` (the same product the
/// per-slot premium integrates over), and the returned value is in
/// `PREMIUM_SCALE` units so it plugs straight into
/// [`compute_premium_per_slot`]'s `base_rate` slot.
///
/// CALIBRATION REQUIRED: this is a first-order point estimate from realized
/// experience. A production calibration should instead target a ruin
/// probability against a fitted loss *distribution* (not just the empirical
/// mean) and re-estimate on a rolling window; this helper documents the
/// mechanism and provides the break-even anchor.
pub fn calibrate_base_rate(
    target_loss_ratio_num: u128,
    target_loss_ratio_den: u128,
    cumulative_claims: u128,
    observed_exposure: u128,
) -> u128 {
    if observed_exposure == 0
        || cumulative_claims == 0
        || target_loss_ratio_num == 0
        || target_loss_ratio_den == 0
    {
        return 0;
    }

    // base_rate = claims * PREMIUM_SCALE * target_den / (target_num * exposure)
    //
    // Reduce by GCD where possible to keep the products in range, mirroring the
    // overflow-safe style used elsewhere in this module.
    let g1 = gcd(cumulative_claims, observed_exposure);
    let claims_r = cumulative_claims / g1;
    let exposure_r = observed_exposure / g1;

    let g2 = gcd(target_loss_ratio_num, target_loss_ratio_den);
    let t_num = target_loss_ratio_num / g2;
    let t_den = target_loss_ratio_den / g2;

    // numerator = claims_r * PREMIUM_SCALE * t_den
    // denominator = exposure_r * t_num
    // Use saturating products; if the numerator overflows we divide first.
    let denom = exposure_r.saturating_mul(t_num);
    if denom == 0 {
        return 0;
    }

    match claims_r
        .checked_mul(PREMIUM_SCALE)
        .and_then(|v| v.checked_mul(t_den))
    {
        Some(numer) => numer / denom,
        None => {
            // Divide first to avoid overflow (slight precision loss, safe).
            let partial = claims_r.saturating_mul(PREMIUM_SCALE) / denom.max(1);
            partial.saturating_mul(t_den)
        }
    }
}

// ============================================================================
// Interval premium (time-integrated)
// ============================================================================

/// Premium for an accrual interval, integrating the system risk over time.
///
/// ```text
/// premium = notional × base_rate × lev_charged × crowd × system_accrued
///           ÷ (PREMIUM_SCALE × MULT_SCALE³)
/// ```
/// `lev_charged` and `crowd` are in MULT_SCALE units; `system_accrued` is
/// `Σ system_index_scaled · dt` (carrying one MULT_SCALE). For a constant system
/// index and leverage this equals `compute_premium_per_slot × slots`. Floored at
/// `min_premium`; saturates UP on true overflow.
pub fn compute_interval_premium(
    notional: u128,
    base_rate: u128,
    lev_charged: u128,
    crowd: u128,
    system_accrued: u128,
    min_premium: u128,
) -> u128 {
    if notional == 0 || system_accrued == 0 {
        return min_premium;
    }
    let den0 = PREMIUM_SCALE
        .saturating_mul(MULT_SCALE)
        .saturating_mul(MULT_SCALE)
        .saturating_mul(MULT_SCALE);

    let mut num: u128 = notional;
    let mut den: u128 = den0;

    for factor in [base_rate, lev_charged, crowd, system_accrued] {
        let g = gcd(num, den);
        num /= g;
        den /= g;
        num = match num.checked_mul(factor) {
            Some(v) => v,
            None => {
                let bits = 128u32 - num.leading_zeros();
                let shift = bits.saturating_sub(64);
                num >>= shift;
                den >>= shift;
                match num.checked_mul(factor) {
                    Some(v) => v,
                    None => return u128::MAX,
                }
            }
        };
    }

    let g = gcd(num, den);
    let num = num / g;
    let den = den / g;
    if den == 0 {
        return u128::MAX;
    }
    let result = match num.checked_add(den - 1) {
        Some(v) => v / den,
        None => num / den,
    };
    result.max(min_premium)
}

// ============================================================================
// Full premium calculation
// ============================================================================

/// Compute the per-slot insurance premium for one account.
///
/// # Formula
/// ```text
/// premium = notional × base_rate × lev_num × crowd_num × oiv_num × pool_num × vol_num × tail_num
///           ÷ (PREMIUM_SCALE × lev_den × crowd_den × oiv_den × pool_den × vol_den × tail_den)
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
    // Task 2: realized-volatility multiplier (gap-risk scaling). Folded in
    // identically to the other multipliers; neutral leaves the premium
    // unchanged.
    let (vol_num, vol_den) = risk_idx.volatility;
    // Task 3: leverage tail surcharge, applied on top of the base leverage^1.5
    // factor. Neutral when the position is below the maintenance-proximity
    // threshold or the surcharge is disabled.
    let (tail_num, tail_den) = risk_idx.leverage_tail;

    // Build numerator: notional × base_rate × lev_num × crowd_num × oiv_num × pool_num
    // Use GCD reduction at each step to prevent overflow.
    let mut num: u128 = notional;
    let mut den: u128 = PREMIUM_SCALE;

    // Multiply in base_rate.
    //
    // If `num × base_rate` overflows even after GCD-reducing `num` against
    // `den`, the true numerator already exceeds u128::MAX. Because every
    // canonical risk multiplier is ≥ 1.0 (each floors at `MULT_SCALE`), the
    // final premium is then ≥ this saturated numerator divided by `den`, which
    // is itself far above any representable premium. The CONSERVATIVE answer is
    // therefore the maximum representable premium — we short-circuit to it.
    //
    // The previous fallback (`num = num / base_rate.max(1) * base_rate`) floored
    // to ~num (silently DROPPING the `× base_rate` factor) or, when
    // `num < base_rate`, to 0 (collapsing the entire numerator) — under-pricing
    // by a factor of `base_rate`. That is the bug this fixes (engineer #4).
    num = match num.checked_mul(base_rate) {
        Some(v) => v,
        None => {
            // Reduce first.
            let g = gcd(num, den);
            num /= g;
            den /= g;
            match num.checked_mul(base_rate) {
                Some(v) => v,
                None => {
                    // Still overflows → true product > u128::MAX. Saturate the
                    // PREMIUM upward and return immediately (conservative
                    // over-pricing), never silently dropping the factor.
                    // u128::MAX already dominates `min_premium`, so it is the
                    // floored result.
                    return u128::MAX;
                }
            }
        }
    };

    // Helper macro replaced by inline function pattern — reduce and multiply
    // for each multiplier component.
    macro_rules! mul_component {
        ($num:expr, $den:expr, $c_num:expr, $c_den:expr, $bail:expr) => {{
            let g = gcd($num, $den);
            $num /= g;
            $den /= g;
            let g2 = gcd($num, $c_den);
            $num /= g2;
            let c_den_r = $c_den / g2;
            let g3 = gcd($c_num, $den);
            let c_num_r = $c_num / g3;
            $den /= g3;
            $den = match $den.checked_mul(c_den_r) {
                Some(v) => v,
                None => return $bail,
            };
            $num = match $num.checked_mul(c_num_r) {
                Some(v) => v,
                None => {
                    let bits = 128u32 - $num.leading_zeros();
                    let shift = bits.saturating_sub(64);
                    $num >>= shift;
                    $den >>= shift;
                    match $num.checked_mul(c_num_r) {
                        Some(v) => v,
                        None => return $bail,
                    }
                }
            };
        }};
    }

    // Bail value on a multiplier-fold overflow is `u128::MAX` (saturate UP), not
    // `min_premium`. Every factor folded here is >= 1.0 (each canonical
    // multiplier floors at MULT_SCALE; the leverage/tail/vol factors are >= 1.0),
    // so folding one can only RAISE the premium. Collapsing to `min_premium` on
    // overflow would under-price and break monotonicity at the boundary
    // (review #5). u128::MAX is the conservative, monotonic answer — consistent
    // with the base_rate overflow fix above. Triggers only at astronomical,
    // non-economic magnitudes.
    mul_component!(num, den, lev_num, lev_den, u128::MAX);
    mul_component!(num, den, crowd_num, crowd_den, u128::MAX);
    mul_component!(num, den, oiv_num, oiv_den, u128::MAX);
    mul_component!(num, den, pool_num, pool_den, u128::MAX);
    mul_component!(num, den, vol_num, vol_den, u128::MAX);
    mul_component!(num, den, tail_num, tail_den, u128::MAX);

    // Final GCD reduction
    let g = gcd(num, den);
    let num = num / g;
    let den = den / g;

    // `den` starts at PREMIUM_SCALE and only ever shrinks via exact GCD division
    // (which keeps it >= 1) — EXCEPT the overflow `den >>= shift` fallback in
    // mul_component!, which can underflow it to 0. So `den == 0` implies an
    // overflow occurred and the true premium is astronomically large: saturate
    // UP (review #5), consistent with the other overflow paths. Never collapse
    // to min_premium here.
    if den == 0 {
        return u128::MAX;
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
