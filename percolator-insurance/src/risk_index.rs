//! Systemic risk index computed from on-chain signals.
//!
//! Provides pure functions that compute risk multipliers from on-chain state.
//! All multipliers are `(num, den)` pairs where `(MULT_SCALE, MULT_SCALE)`
//! represents 1.0x. No floating point. `no_std` compatible.

use crate::MULT_SCALE;

// ============================================================================
// RiskIndex struct
// ============================================================================

/// Aggregated risk multipliers derived from on-chain market state.
///
/// Each field is a `(num, den)` pair. The effective multiplier is `num / den`.
/// A value of `(MULT_SCALE, MULT_SCALE)` represents a neutral 1.0x multiplier.
#[derive(Clone, Copy, Debug)]
pub struct RiskIndex {
    /// Multiplier from OI crowding on the majority side.
    pub crowding: (u128, u128),
    /// Multiplier from system-level leverage (total OI vs vault TVL).
    pub oi_vault: (u128, u128),
    /// Multiplier from insurance pool depletion.
    pub pool_health: (u128, u128),
}

impl RiskIndex {
    /// Returns a neutral `RiskIndex` with all multipliers at 1.0x.
    pub fn neutral() -> Self {
        Self {
            crowding: (MULT_SCALE, MULT_SCALE),
            oi_vault: (MULT_SCALE, MULT_SCALE),
            pool_health: (MULT_SCALE, MULT_SCALE),
        }
    }
}

// ============================================================================
// crowding_multiplier
// ============================================================================

/// Penalizes accounts on the dominant OI side.
///
/// Returns `(num, MULT_SCALE)` where `num / MULT_SCALE` is the multiplier.
///
/// # Parameters
/// - `oi_majority`: larger OI side
/// - `oi_minority`: smaller OI side
/// - `is_majority_side`: whether this account is on the crowded side
/// - `low_ratio_num / low_ratio_den`: ratio threshold below which multiplier is 1.0
/// - `high_ratio_num / high_ratio_den`: ratio above which multiplier is capped
/// - `cap`: maximum multiplier scaled by MULT_SCALE
///
/// # Logic
/// - Minority side → always 1.0
/// - `oi_minority == 0` → cap
/// - `ratio <= low_threshold` → 1.0
/// - `ratio >= high_threshold` → cap
/// - Otherwise → linear interpolation between 1.0 and cap
pub fn crowding_multiplier(
    oi_majority: u128,
    oi_minority: u128,
    is_majority_side: bool,
    low_ratio_num: u128,
    low_ratio_den: u128,
    high_ratio_num: u128,
    high_ratio_den: u128,
    cap: u128,
) -> (u128, u128) {
    const ONE: (u128, u128) = (MULT_SCALE, MULT_SCALE);

    if !is_majority_side {
        return ONE;
    }

    if oi_minority == 0 {
        return (cap, MULT_SCALE);
    }

    // Compare ratio = oi_majority / oi_minority  vs  low_ratio_num / low_ratio_den
    // ratio <= low  ⟺  oi_majority * low_ratio_den <= oi_minority * low_ratio_num
    if oi_majority.saturating_mul(low_ratio_den) <= oi_minority.saturating_mul(low_ratio_num) {
        return ONE;
    }

    // ratio >= high  ⟺  oi_majority * high_ratio_den >= oi_minority * high_ratio_num
    if oi_majority.saturating_mul(high_ratio_den) >= oi_minority.saturating_mul(high_ratio_num) {
        return (cap, MULT_SCALE);
    }

    // Linear interpolation:
    //   ratio_scaled = oi_majority * 1000 / oi_minority  (in MULT_SCALE units)
    //   low_scaled   = low_ratio_num  * MULT_SCALE / low_ratio_den
    //   high_scaled  = high_ratio_num * MULT_SCALE / high_ratio_den
    //
    //   position = (ratio_scaled - low_scaled) / (high_scaled - low_scaled)
    //   mult = MULT_SCALE + position * (cap - MULT_SCALE)

    let ratio_scaled = oi_majority.saturating_mul(MULT_SCALE) / oi_minority;
    let low_scaled = low_ratio_num.saturating_mul(MULT_SCALE) / low_ratio_den;
    let high_scaled = high_ratio_num.saturating_mul(MULT_SCALE) / high_ratio_den;

    let range = high_scaled.saturating_sub(low_scaled);
    if range == 0 {
        return (cap, MULT_SCALE);
    }

    let delta = ratio_scaled.saturating_sub(low_scaled);
    // mult = MULT_SCALE + delta * (cap - MULT_SCALE) / range
    let spread = cap.saturating_sub(MULT_SCALE);
    let num = MULT_SCALE + delta.saturating_mul(spread) / range;

    (num, MULT_SCALE)
}

// ============================================================================
// oi_vault_multiplier
// ============================================================================

/// Measures system-level leverage (total OI notional vs vault TVL).
///
/// Returns `(num, MULT_SCALE)`.
///
/// # Parameters
/// - `total_oi_notional`: `(oi_long + oi_short) * oracle_price / POS_SCALE`
/// - `vault`: vault balance
/// - `floor_ratio_num / floor_ratio_den`: below this leverage → 1.0
/// - `cap_ratio_num / cap_ratio_den`: above this leverage → `mult_max`
/// - `mult_max`: maximum multiplier scaled by MULT_SCALE
///
/// Same interpolation pattern as `crowding_multiplier`.
pub fn oi_vault_multiplier(
    total_oi_notional: u128,
    vault: u128,
    floor_ratio_num: u128,
    floor_ratio_den: u128,
    cap_ratio_num: u128,
    cap_ratio_den: u128,
    mult_max: u128,
) -> (u128, u128) {
    const ONE: (u128, u128) = (MULT_SCALE, MULT_SCALE);

    if vault == 0 {
        return (mult_max, MULT_SCALE);
    }

    // Compare sys_lev = total_oi_notional / vault  vs  floor_ratio
    // sys_lev <= floor  ⟺  total_oi_notional * floor_den <= vault * floor_num
    if total_oi_notional.saturating_mul(floor_ratio_den)
        <= vault.saturating_mul(floor_ratio_num)
    {
        return ONE;
    }

    // sys_lev >= cap_ratio  ⟺  total_oi_notional * cap_den >= vault * cap_num
    if total_oi_notional.saturating_mul(cap_ratio_den)
        >= vault.saturating_mul(cap_ratio_num)
    {
        return (mult_max, MULT_SCALE);
    }

    // Linear interpolation in MULT_SCALE-scaled space
    let ratio_scaled = total_oi_notional.saturating_mul(MULT_SCALE) / vault;
    let floor_scaled = floor_ratio_num.saturating_mul(MULT_SCALE) / floor_ratio_den;
    let cap_scaled = cap_ratio_num.saturating_mul(MULT_SCALE) / cap_ratio_den;

    let range = cap_scaled.saturating_sub(floor_scaled);
    if range == 0 {
        return (mult_max, MULT_SCALE);
    }

    let delta = ratio_scaled.saturating_sub(floor_scaled);
    let spread = mult_max.saturating_sub(MULT_SCALE);
    let num = MULT_SCALE + delta.saturating_mul(spread) / range;

    (num, MULT_SCALE)
}

// ============================================================================
// pool_health_multiplier
// ============================================================================

/// Spikes when the insurance pool is depleted relative to total OI notional.
///
/// Returns `(num, MULT_SCALE)`. Inverted interpolation — lower health = higher
/// multiplier.
///
/// # Parameters
/// - `pool_balance`: current insurance pool balance
/// - `total_oi_notional`: total open interest notional
/// - `low_health_num / low_health_den`: health below this → `mult_max`
/// - `high_health_num / high_health_den`: health above this → 1.0
/// - `mult_max`: maximum multiplier scaled by MULT_SCALE
///
/// # Logic
/// - `total_oi_notional == 0` → 1.0 (no risk)
/// - `health >= high` → 1.0
/// - `health <= low` → `mult_max`
/// - Otherwise → `mult_max - position * (mult_max - MULT_SCALE)` where
///   `position` goes from 0 at low_health to 1 at high_health
pub fn pool_health_multiplier(
    pool_balance: u128,
    total_oi_notional: u128,
    low_health_num: u128,
    low_health_den: u128,
    high_health_num: u128,
    high_health_den: u128,
    mult_max: u128,
) -> (u128, u128) {
    const ONE: (u128, u128) = (MULT_SCALE, MULT_SCALE);

    if total_oi_notional == 0 {
        return ONE;
    }

    // Compare health = pool_balance / total_oi_notional  vs  high_health
    // health >= high  ⟺  pool_balance * high_den >= total_oi_notional * high_num
    if pool_balance.saturating_mul(high_health_den)
        >= total_oi_notional.saturating_mul(high_health_num)
    {
        return ONE;
    }

    // health <= low  ⟺  pool_balance * low_den <= total_oi_notional * low_num
    if pool_balance.saturating_mul(low_health_den)
        <= total_oi_notional.saturating_mul(low_health_num)
    {
        return (mult_max, MULT_SCALE);
    }

    // Inverted linear interpolation.
    // Use health_scaled = pool_balance * 10_000 / total_oi_notional for precision.
    // low_scaled and high_scaled are in the same 10_000-denominated space.
    const HEALTH_SCALE: u128 = 10_000;

    let health_scaled = pool_balance.saturating_mul(HEALTH_SCALE) / total_oi_notional;
    let low_scaled = low_health_num.saturating_mul(HEALTH_SCALE) / low_health_den;
    let high_scaled = high_health_num.saturating_mul(HEALTH_SCALE) / high_health_den;

    let range = high_scaled.saturating_sub(low_scaled);
    if range == 0 {
        return (mult_max, MULT_SCALE);
    }

    // position = (health_scaled - low_scaled) / range   [0 at low, 1 at high]
    let delta = health_scaled.saturating_sub(low_scaled);
    let spread = mult_max.saturating_sub(MULT_SCALE);

    // mult = mult_max - position * spread
    //      = mult_max - (delta * spread / range)
    let reduction = delta.saturating_mul(spread) / range;
    let num = mult_max.saturating_sub(reduction);

    (num, MULT_SCALE)
}
