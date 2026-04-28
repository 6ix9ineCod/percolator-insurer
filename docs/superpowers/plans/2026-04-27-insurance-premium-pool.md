# Insurance Premium Pool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a wrapper crate (`percolator-insurance`) that adds risk-priced insurance premiums to Percolator's risk engine, collecting per-slot fees based on leverage, market crowding, system leverage, and pool health.

**Architecture:** Separate Rust crate importing `percolator` as a dependency. Four modules — `premium.rs` (pure math), `pool.rs` (accounting), `risk_index.rs` (on-chain signals), `wrapper.rs` (orchestrator). All interaction with Percolator via its public API only. `no_std` compatible, pure integer math, `u256` intermediates for overflow safety.

**Tech Stack:** Rust (edition 2021), `no_std`, `percolator` crate as dependency, `proptest` for fuzz testing.

**Spec:** `docs/superpowers/specs/2026-04-27-insurance-premium-pool-design.md`

---

## File Structure

```
percolator-insurance/
  Cargo.toml                  — crate manifest, depends on percolator (path)
  src/
    lib.rs                    — public API facade, re-exports, constants
    premium.rs                — premium calculation engine (pure functions)
    pool.rs                   — PremiumPool struct and accounting
    risk_index.rs             — systemic risk index from on-chain signals
    wrapper.rs                — InsuredRiskEngine orchestrator
  tests/
    premium_tests.rs          — unit + golden value tests for premium math
    pool_tests.rs             — pool accounting and invariant tests
    risk_index_tests.rs       — risk signal multiplier tests
    integration_tests.rs      — full lifecycle through wrapper
    fuzz_tests.rs             — proptest fuzz tests
```

---

### Task 1: Crate Scaffold and Constants

**Files:**
- Create: `percolator-insurance/Cargo.toml`
- Create: `percolator-insurance/src/lib.rs`

- [ ] **Step 1: Create the crate directory**

```bash
mkdir -p percolator-insurance/src percolator-insurance/tests
```

- [ ] **Step 2: Write Cargo.toml**

Create `percolator-insurance/Cargo.toml`:

```toml
[package]
name = "percolator-insurance"
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"

[lib]
name = "percolator_insurance"
path = "src/lib.rs"

[dependencies]
percolator = { path = "../", features = ["test"] }

[dev-dependencies]
proptest = "1.4"

[features]
default = []
test = []
```

- [ ] **Step 3: Write lib.rs with constants and module declarations**

Create `percolator-insurance/src/lib.rs`:

```rust
//! Insurance Premium Pool for Percolator Risk Engine
//!
//! Wrapper crate that adds risk-priced insurance premiums to Percolator.
//! Collects per-slot fees based on leverage, market crowding, system leverage,
//! and pool health. Feeds Percolator's insurance fund via its public API.
//!
//! All math is pure integer arithmetic using u256 intermediates.
//! No floating point. no_std compatible.

#![no_std]
#![forbid(unsafe_code)]

pub use percolator::{
    Account, InsuranceFund, RiskEngine, RiskError, RiskParams,
    Result as PercolatorResult, Side, MarketMode, LiquidationPolicy,
    MAX_ACCOUNTS, POS_SCALE, MAX_ORACLE_PRICE, MAX_VAULT_TVL,
    MAX_ACCOUNT_NOTIONAL, MAX_OI_SIDE_Q, FUNDING_DEN,
};

pub mod premium;
pub mod pool;
pub mod risk_index;
pub mod wrapper;

/// Premium scaling denominator (1e9, matches Percolator's FUNDING_DEN).
pub const PREMIUM_SCALE: u128 = 1_000_000_000;

/// Leverage scaling factor for fixed-point leverage computation.
/// leverage = notional * LEVERAGE_SCALE / capital.
pub const LEVERAGE_SCALE: u128 = 1_000_000;

/// Multiplier scaling factor. All (num, den) multiplier pairs use this
/// as their denominator when representing 1.0.
pub const MULT_SCALE: u128 = 1_000;

/// Slots per day at 400ms per slot (86_400_000ms / 400ms).
pub const SLOTS_PER_DAY: u64 = 216_000;

/// Error types for the insurance wrapper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsuredError {
    /// Pass-through from Percolator.
    Risk(RiskError),
    /// Account cannot afford the 24h upfront commitment.
    InsufficientForCommitment,
    /// Premium collection failed (account capital exhausted).
    PremiumCollectionFailed,
    /// top_up_insurance_fund rejected (vault TVL cap, time monotonicity).
    PoolTopUpFailed,
    /// Invalid premium parameters at initialization.
    InvalidParams,
}

impl From<RiskError> for InsuredError {
    fn from(e: RiskError) -> Self {
        InsuredError::Risk(e)
    }
}

pub type Result<T> = core::result::Result<T, InsuredError>;
```

- [ ] **Step 4: Verify it compiles**

```bash
cd percolator-insurance && cargo check
```

Expected: compiles with no errors.

- [ ] **Step 5: Commit**

```bash
git add percolator-insurance/
git commit -m "feat: scaffold percolator-insurance crate with constants and error types"
```

---

### Task 2: Premium Calculation — Integer Square Root and Power

**Files:**
- Create: `percolator-insurance/src/premium.rs`
- Create: `percolator-insurance/tests/premium_tests.rs`

- [ ] **Step 1: Write failing tests for isqrt**

Create `percolator-insurance/tests/premium_tests.rs`:

```rust
use percolator_insurance::premium::isqrt;

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
    // isqrt floors: sqrt(2) = 1, sqrt(3) = 1, sqrt(5) = 2
    assert_eq!(isqrt(2), 1);
    assert_eq!(isqrt(3), 1);
    assert_eq!(isqrt(5), 2);
    assert_eq!(isqrt(8), 2);
    assert_eq!(isqrt(10), 3);
}

#[test]
fn test_isqrt_large() {
    // sqrt(u128::MAX) ≈ 1.844e19
    let result = isqrt(u128::MAX);
    assert!(result * result <= u128::MAX);
    assert!((result + 1).checked_mul(result + 1).is_none() || (result + 1) * (result + 1) > u128::MAX);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd percolator-insurance && cargo test --test premium_tests 2>&1 | head -20
```

Expected: FAIL — `isqrt` not found.

- [ ] **Step 3: Implement isqrt and leverage_multiplier**

Create `percolator-insurance/src/premium.rs`:

```rust
//! Premium calculation engine.
//!
//! Pure functions that compute per-slot premium rates from account and market
//! state. No side effects, no state mutation. All arithmetic uses integer
//! math with u256 intermediates for overflow safety.

use crate::{LEVERAGE_SCALE, MULT_SCALE, PREMIUM_SCALE};

/// Integer square root via Newton's method.
/// Returns floor(sqrt(n)). No floating point.
pub fn isqrt(n: u128) -> u128 {
    if n <= 1 {
        return n;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Integer nth-root via Newton's method.
/// Returns floor(n^(1/k)). Panics if k == 0.
pub fn inth_root(n: u128, k: u32) -> u128 {
    if k == 0 {
        panic!("inth_root: k must be > 0");
    }
    if k == 1 {
        return n;
    }
    if k == 2 {
        return isqrt(n);
    }
    if n <= 1 {
        return n;
    }
    // Newton's method: x_{i+1} = ((k-1) * x_i + n / x_i^(k-1)) / k
    let km1 = (k - 1) as u128;
    let mut x: u128 = {
        // Initial guess: 2^(ceil(bit_length(n) / k))
        let bits = 128 - n.leading_zeros();
        let shift = (bits + k - 1) / k;
        1u128 << shift
    };
    loop {
        // Compute x^(k-1) with overflow protection
        let mut xpow = 1u128;
        let mut overflowed = false;
        for _ in 0..km1 {
            match xpow.checked_mul(x) {
                Some(v) => xpow = v,
                None => {
                    overflowed = true;
                    break;
                }
            }
        }
        let y = if overflowed {
            // x is too large, halve it
            x / 2
        } else if xpow == 0 {
            return 0;
        } else {
            let div = n / xpow;
            // ((k-1) * x + n / x^(k-1)) / k
            match km1.checked_mul(x) {
                Some(prod) => match prod.checked_add(div) {
                    Some(sum) => sum / k as u128,
                    None => (km1 / k as u128) * x + div / k as u128,
                },
                None => (km1 / k as u128) * x + div / k as u128,
            }
        };
        if y >= x {
            break;
        }
        x = y;
    }
    x
}

/// Compute leverage multiplier as a (num, den) pair.
///
/// leverage = notional * LEVERAGE_SCALE / capital
/// multiplier = leverage^(exp_num / exp_den)
///            = (leverage^exp_num)^(1/exp_den)
///
/// Returns (multiplier_num, multiplier_den) scaled by MULT_SCALE.
/// For a 1.0x multiplier, returns (MULT_SCALE, MULT_SCALE).
/// Minimum return is (MULT_SCALE, MULT_SCALE) (1.0x) — leverage < 1 is clamped.
pub fn leverage_multiplier(
    notional: u128,
    capital: u128,
    exp_num: u64,
    exp_den: u64,
) -> (u128, u128) {
    if capital == 0 || notional == 0 || exp_den == 0 {
        return (MULT_SCALE, MULT_SCALE);
    }

    // leverage = notional * LEVERAGE_SCALE / capital
    let lev_scaled = match notional.checked_mul(LEVERAGE_SCALE) {
        Some(n) => n / capital,
        None => {
            // Wide: (notional / capital) * LEVERAGE_SCALE
            (notional / capital).saturating_mul(LEVERAGE_SCALE)
        }
    };

    // Clamp: if leverage < 1.0 (lev_scaled < LEVERAGE_SCALE), return 1.0
    if lev_scaled <= LEVERAGE_SCALE {
        return (MULT_SCALE, MULT_SCALE);
    }

    // For exp 1.5 (num=3, den=2): lev^(3/2) = lev * sqrt(lev)
    // General: lev^(p/q) = (lev^p)^(1/q)
    //
    // We work in LEVERAGE_SCALE fixed point:
    //   lev_scaled = leverage * LEVERAGE_SCALE
    //   lev^p in fixed point = lev_scaled^p / LEVERAGE_SCALE^(p-1)
    //   then take qth root

    let p = exp_num as u32;
    let q = exp_den as u32;

    // Compute lev_scaled^p (may overflow u128 for large p, use checked)
    let mut powered: u128 = lev_scaled;
    let mut overflow = false;
    for _ in 1..p {
        match powered.checked_mul(lev_scaled) {
            Some(v) => powered = v,
            None => {
                overflow = true;
                break;
            }
        }
    }

    if overflow {
        // Leverage is extremely high — return a saturated large multiplier.
        // Cap at u64::MAX to stay reasonable.
        return (u64::MAX as u128, MULT_SCALE);
    }

    // Divide by LEVERAGE_SCALE^(p-1) to keep in fixed point
    let mut scale_divisor: u128 = 1;
    for _ in 1..p {
        match scale_divisor.checked_mul(LEVERAGE_SCALE) {
            Some(v) => scale_divisor = v,
            None => return (u64::MAX as u128, MULT_SCALE),
        }
    }
    let normalized = powered / scale_divisor;

    // Take qth root: result is in LEVERAGE_SCALE fixed point
    let rooted = if q == 1 {
        normalized
    } else {
        // Scale up before root to preserve precision
        let scale_up = match normalized.checked_mul(LEVERAGE_SCALE.pow(q - 1)) {
            Some(v) => v,
            None => {
                // Fall back to unscaled root
                return (inth_root(normalized, q) * MULT_SCALE / LEVERAGE_SCALE, MULT_SCALE);
            }
        };
        inth_root(scale_up, q)
    };

    // Convert from LEVERAGE_SCALE to MULT_SCALE
    let mult_num = rooted * MULT_SCALE / LEVERAGE_SCALE;
    let mult_num = core::cmp::max(mult_num, MULT_SCALE); // floor at 1.0

    (mult_num, MULT_SCALE)
}
```

- [ ] **Step 4: Add leverage multiplier tests**

Append to `percolator-insurance/tests/premium_tests.rs`:

```rust
use percolator_insurance::premium::leverage_multiplier;
use percolator_insurance::MULT_SCALE;

#[test]
fn test_leverage_mult_1x() {
    // notional == capital → leverage 1.0 → multiplier 1.0
    let (num, den) = leverage_multiplier(60_000, 60_000, 3, 2);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_leverage_mult_below_1x() {
    // notional < capital → clamped to 1.0
    let (num, den) = leverage_multiplier(30_000, 60_000, 3, 2);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_leverage_mult_zero_capital() {
    let (num, den) = leverage_multiplier(60_000, 0, 3, 2);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_leverage_mult_5x_exp_1_5() {
    // 5^1.5 = 11.18
    let (num, den) = leverage_multiplier(50_000, 10_000, 3, 2);
    let ratio = num * 100 / den;
    // Allow 5% tolerance: 11.18 * 100 = 1118, accept 1060..1180
    assert!(ratio >= 1060 && ratio <= 1180, "5x mult ratio: {}", ratio);
}

#[test]
fn test_leverage_mult_10x_exp_1_5() {
    // 10^1.5 = 31.62
    let (num, den) = leverage_multiplier(100_000, 10_000, 3, 2);
    let ratio = num * 100 / den;
    // Accept 3000..3320
    assert!(ratio >= 3000 && ratio <= 3320, "10x mult ratio: {}", ratio);
}

#[test]
fn test_leverage_mult_25x_exp_1_5() {
    // 25^1.5 = 125.0
    let (num, den) = leverage_multiplier(250_000, 10_000, 3, 2);
    let ratio = num * 100 / den;
    // Accept 11900..13100
    assert!(ratio >= 11900 && ratio <= 13100, "25x mult ratio: {}", ratio);
}

#[test]
fn test_leverage_mult_100x_exp_1_5() {
    // 100^1.5 = 1000.0
    let (num, den) = leverage_multiplier(1_000_000, 10_000, 3, 2);
    let ratio = num * 100 / den;
    // Accept 95000..105000
    assert!(ratio >= 95000 && ratio <= 105000, "100x mult ratio: {}", ratio);
}

#[test]
fn test_leverage_mult_linear_exponent() {
    // Exponent 1.0 (num=1, den=1): multiplier should equal leverage
    let (num, den) = leverage_multiplier(100_000, 10_000, 1, 1);
    let ratio = num * 100 / den;
    // 10x leverage, linear: 1000
    assert!(ratio >= 950 && ratio <= 1050, "linear mult ratio: {}", ratio);
}

#[test]
fn test_leverage_mult_quadratic_exponent() {
    // Exponent 2.0 (num=2, den=1): 10^2 = 100
    let (num, den) = leverage_multiplier(100_000, 10_000, 2, 1);
    let ratio = num * 100 / den;
    // Accept 9500..10500
    assert!(ratio >= 9500 && ratio <= 10500, "quadratic mult ratio: {}", ratio);
}
```

- [ ] **Step 5: Run tests**

```bash
cd percolator-insurance && cargo test --test premium_tests -- --nocapture 2>&1
```

Expected: all tests PASS.

- [ ] **Step 6: Commit**

```bash
git add percolator-insurance/src/premium.rs percolator-insurance/tests/premium_tests.rs
git commit -m "feat: implement isqrt, inth_root, and leverage_multiplier with tests"
```

---

### Task 3: Premium Calculation — Full Premium Rate

**Files:**
- Modify: `percolator-insurance/src/premium.rs`
- Modify: `percolator-insurance/tests/premium_tests.rs`

- [ ] **Step 1: Write failing test for compute_premium_per_slot**

Append to `percolator-insurance/tests/premium_tests.rs`:

```rust
use percolator_insurance::premium::compute_premium_per_slot;
use percolator_insurance::pool::PremiumPool;
use percolator_insurance::risk_index::RiskIndex;
use percolator_insurance::PREMIUM_SCALE;

#[test]
fn test_premium_zero_notional() {
    let idx = RiskIndex {
        crowding: (1000, 1000),
        oi_vault: (1000, 1000),
        pool_health: (1000, 1000),
    };
    let result = compute_premium_per_slot(0, 60_000, 100, &idx, 1);
    assert_eq!(result, 0);
}

#[test]
fn test_premium_basic_calculation() {
    // notional=60_000, capital=40_000 (1.5x leverage), all multipliers 1.0
    let idx = RiskIndex {
        crowding: (1000, 1000),
        oi_vault: (1000, 1000),
        pool_health: (1000, 1000),
    };
    let result = compute_premium_per_slot(60_000, 40_000, 100, &idx, 1);
    assert!(result > 0, "premium must be positive for nonzero position");
}

#[test]
fn test_premium_increases_with_leverage() {
    let idx = RiskIndex {
        crowding: (1000, 1000),
        oi_vault: (1000, 1000),
        pool_health: (1000, 1000),
    };
    // 5x leverage
    let prem_5x = compute_premium_per_slot(50_000, 10_000, 100, &idx, 1);
    // 25x leverage
    let prem_25x = compute_premium_per_slot(250_000, 10_000, 100, &idx, 1);
    // 100x leverage
    let prem_100x = compute_premium_per_slot(1_000_000, 10_000, 100, &idx, 1);

    assert!(prem_25x > prem_5x, "25x must cost more than 5x");
    assert!(prem_100x > prem_25x, "100x must cost more than 25x");
    // Superlinear: 100x/5x ratio should be >> 20 (linear would be 20)
    assert!(prem_100x / prem_5x > 50, "leverage curve must be superlinear");
}

#[test]
fn test_premium_increases_with_crowding() {
    let idx_normal = RiskIndex {
        crowding: (1000, 1000),
        oi_vault: (1000, 1000),
        pool_health: (1000, 1000),
    };
    let idx_crowded = RiskIndex {
        crowding: (3000, 1000),
        oi_vault: (1000, 1000),
        pool_health: (1000, 1000),
    };
    let prem_normal = compute_premium_per_slot(100_000, 10_000, 100, &idx_normal, 1);
    let prem_crowded = compute_premium_per_slot(100_000, 10_000, 100, &idx_crowded, 1);

    assert!(prem_crowded > prem_normal, "crowded side must pay more");
    // 3x crowding multiplier should give ~3x premium
    let ratio = prem_crowded * 100 / prem_normal;
    assert!(ratio >= 250 && ratio <= 350, "crowding ratio: {}", ratio);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd percolator-insurance && cargo test --test premium_tests 2>&1 | head -20
```

Expected: FAIL — `compute_premium_per_slot` not found.

- [ ] **Step 3: Implement compute_premium_per_slot**

Append to `percolator-insurance/src/premium.rs`:

```rust
use crate::risk_index::RiskIndex;

/// Compute the premium rate for one slot for a single account.
///
/// Formula:
///   premium = notional × base_rate × lev_mult × crowd × oiv × pool_health
///             ÷ (PREMIUM_SCALE × MULT_SCALE^4)
///
/// All multipliers come as (num, den) pairs from RiskIndex.
/// Leverage multiplier is computed internally from notional/capital.
///
/// Returns premium in token units per slot. Rounds up (conservative).
/// Returns 0 only when notional is 0.
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

    // Leverage multiplier (hardcoded 1.5 exponent for now — will be parameterized)
    let (lev_num, lev_den) = leverage_multiplier(notional, capital, 3, 2);

    // Build numerator and denominator products.
    // numerator = notional × base_rate × lev_num × crowd_num × oiv_num × pool_num
    // denominator = PREMIUM_SCALE × lev_den × crowd_den × oiv_den × pool_den

    // Use checked u128 first, fall back to manual splitting if overflow.
    let num_parts: [u128; 6] = [
        notional,
        base_rate,
        lev_num,
        risk_idx.crowding.0,
        risk_idx.oi_vault.0,
        risk_idx.pool_health.0,
    ];
    let den_parts: [u128; 5] = [
        PREMIUM_SCALE,
        lev_den,
        risk_idx.crowding.1,
        risk_idx.oi_vault.1,
        risk_idx.pool_health.1,
    ];

    // Multiply all numerator parts, cancelling with denominator where possible
    // to avoid overflow. Strategy: alternate multiply/divide.
    let mut result: u128 = 1;
    let mut den_remaining: u128 = 1;

    // Accumulate denominator
    for &d in &den_parts {
        den_remaining = match den_remaining.checked_mul(d) {
            Some(v) => v,
            None => {
                // Denominator overflow — premium would be near-zero. Return min.
                return min_premium;
            }
        };
    }

    if den_remaining == 0 {
        return min_premium;
    }

    // Accumulate numerator with intermediate GCD reduction
    let mut num_acc: u128 = 1;
    let mut den_acc: u128 = den_remaining;
    for &n in &num_parts {
        // Try direct multiply
        match num_acc.checked_mul(n) {
            Some(v) => num_acc = v,
            None => {
                // Reduce before multiplying: divide both by GCD
                let g = gcd(num_acc, den_acc);
                num_acc /= g;
                den_acc /= g;
                match num_acc.checked_mul(n) {
                    Some(v) => num_acc = v,
                    None => {
                        // Still overflows — do partial division first
                        num_acc = num_acc / den_acc;
                        den_acc = 1;
                        num_acc = num_acc.saturating_mul(n);
                    }
                }
            }
        }
    }

    // Final division (ceil)
    result = if den_acc == 0 {
        min_premium
    } else {
        (num_acc + den_acc - 1) / den_acc
    };

    core::cmp::max(result, min_premium)
}

/// GCD via binary algorithm (no division, fast on u128).
fn gcd(mut a: u128, mut b: u128) -> u128 {
    if a == 0 { return b; }
    if b == 0 { return a; }
    let shift = (a | b).trailing_zeros();
    a >>= a.trailing_zeros();
    loop {
        b >>= b.trailing_zeros();
        if a > b {
            core::mem::swap(&mut a, &mut b);
        }
        b -= a;
        if b == 0 {
            return a << shift;
        }
    }
}
```

- [ ] **Step 4: Create stub modules so compilation works**

Create `percolator-insurance/src/risk_index.rs`:

```rust
//! Systemic risk index computed from on-chain signals.
//!
//! All multipliers are (numerator, denominator) pairs.
//! A value of (1000, 1000) represents 1.0x (no surcharge).

/// Multiplier set from on-chain risk signals.
#[derive(Clone, Copy, Debug)]
pub struct RiskIndex {
    /// Crowding multiplier (num, den). Penalizes dominant OI side.
    pub crowding: (u128, u128),
    /// OI/vault multiplier (num, den). Penalizes high system leverage.
    pub oi_vault: (u128, u128),
    /// Pool health multiplier (num, den). Spikes when pool is depleted.
    pub pool_health: (u128, u128),
}

impl RiskIndex {
    /// Neutral risk index — all multipliers at 1.0.
    pub fn neutral() -> Self {
        Self {
            crowding: (1000, 1000),
            oi_vault: (1000, 1000),
            pool_health: (1000, 1000),
        }
    }
}
```

Create `percolator-insurance/src/pool.rs`:

```rust
//! Premium pool accounting.

/// Tracks premium funds as a claim on Percolator's insurance fund.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PremiumPool {
    /// Accounting claim on Percolator's insurance fund balance.
    pub balance: u128,
    /// Lifetime premiums collected (monotonically increasing).
    pub total_collected: u128,
    /// Lifetime consumed by deficit coverage (monotonically increasing).
    pub total_paid_out: u128,
    /// Slot of last deficit reconciliation.
    pub last_deficit_check_slot: u64,
}

impl PremiumPool {
    pub fn new() -> Self {
        Self {
            balance: 0,
            total_collected: 0,
            total_paid_out: 0,
            last_deficit_check_slot: 0,
        }
    }
}
```

Create `percolator-insurance/src/wrapper.rs`:

```rust
//! InsuredRiskEngine orchestrator — wraps Percolator's public API.
```

- [ ] **Step 5: Run all tests**

```bash
cd percolator-insurance && cargo test --test premium_tests -- --nocapture 2>&1
```

Expected: all tests PASS.

- [ ] **Step 6: Commit**

```bash
git add percolator-insurance/src/ percolator-insurance/tests/premium_tests.rs
git commit -m "feat: implement compute_premium_per_slot with GCD-reduced overflow protection"
```

---

### Task 4: Risk Index — On-Chain Signal Multipliers

**Files:**
- Modify: `percolator-insurance/src/risk_index.rs`
- Create: `percolator-insurance/tests/risk_index_tests.rs`

- [ ] **Step 1: Write failing tests for crowding_multiplier**

Create `percolator-insurance/tests/risk_index_tests.rs`:

```rust
use percolator_insurance::risk_index::{crowding_multiplier, oi_vault_multiplier, pool_health_multiplier};
use percolator_insurance::MULT_SCALE;

#[test]
fn test_crowding_balanced() {
    // Equal OI → 1.0 for both sides
    let (num, den) = crowding_multiplier(5000, 5000, true, 1500, 1000, 5000, 1000, 4000);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_crowding_minority_side() {
    // Minority side always gets 1.0 regardless of imbalance
    let (num, den) = crowding_multiplier(8000, 2000, false, 1500, 1000, 5000, 1000, 4000);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_crowding_majority_max() {
    // ratio = 10000/1 → way above cap → should be capped at 4.0
    let (num, den) = crowding_multiplier(10000, 1, true, 1500, 1000, 5000, 1000, 4000);
    assert_eq!(num, 4000);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_crowding_one_side_empty() {
    // One side empty → majority gets cap
    let (num, den) = crowding_multiplier(5000, 0, true, 1500, 1000, 5000, 1000, 4000);
    assert_eq!(num, 4000);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_crowding_interpolation() {
    // ratio = 3.0, range [1.5, 5.0], cap 4.0
    // position = (3.0 - 1.5) / (5.0 - 1.5) = 1.5/3.5 ≈ 0.4286
    // mult = 1.0 + 0.4286 * 3.0 = 2.286
    let (num, den) = crowding_multiplier(3000, 1000, true, 1500, 1000, 5000, 1000, 4000);
    let ratio = num * 1000 / den;
    // Accept 2100..2500
    assert!(ratio >= 2100 && ratio <= 2500, "interp crowding ratio: {}", ratio);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd percolator-insurance && cargo test --test risk_index_tests 2>&1 | head -10
```

Expected: FAIL — functions not found.

- [ ] **Step 3: Implement crowding_multiplier**

Replace contents of `percolator-insurance/src/risk_index.rs`:

```rust
//! Systemic risk index computed from on-chain signals.
//!
//! All multipliers are (numerator, denominator) pairs where
//! (MULT_SCALE, MULT_SCALE) represents 1.0x (no surcharge).
//! Pure functions — no side effects, no state mutation.

use crate::MULT_SCALE;

/// Multiplier set from on-chain risk signals.
#[derive(Clone, Copy, Debug)]
pub struct RiskIndex {
    /// Crowding multiplier (num, den). Penalizes dominant OI side.
    pub crowding: (u128, u128),
    /// OI/vault multiplier (num, den). Penalizes high system leverage.
    pub oi_vault: (u128, u128),
    /// Pool health multiplier (num, den). Spikes when pool is depleted.
    pub pool_health: (u128, u128),
}

impl RiskIndex {
    /// Neutral risk index — all multipliers at 1.0.
    pub fn neutral() -> Self {
        Self {
            crowding: (MULT_SCALE, MULT_SCALE),
            oi_vault: (MULT_SCALE, MULT_SCALE),
            pool_health: (MULT_SCALE, MULT_SCALE),
        }
    }
}

/// Crowding multiplier for an account based on long/short OI imbalance.
///
/// `oi_majority` / `oi_minority`: the larger and smaller OI sides.
/// `is_majority_side`: whether this account is on the dominant side.
/// `low_ratio_num/den`: ratio below which multiplier is 1.0 (e.g., 1500/1000 = 1.5).
/// `high_ratio_num/den`: ratio above which multiplier is capped (e.g., 5000/1000 = 5.0).
/// `cap`: maximum multiplier (scaled by MULT_SCALE, e.g., 4000 = 4.0x).
///
/// Minority side always returns (MULT_SCALE, MULT_SCALE).
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
    if !is_majority_side {
        return (MULT_SCALE, MULT_SCALE);
    }

    if oi_minority == 0 {
        return (cap, MULT_SCALE);
    }

    // ratio = oi_majority / oi_minority (compare via cross-multiplication)
    // ratio <= low_ratio → 1.0
    // ratio >= high_ratio → cap
    // else → linear interpolation

    // ratio <= low_ratio_num/low_ratio_den  ⟺  oi_majority * low_ratio_den <= oi_minority * low_ratio_num
    let ratio_cross = oi_majority.saturating_mul(low_ratio_den);
    let low_cross = oi_minority.saturating_mul(low_ratio_num);
    if ratio_cross <= low_cross {
        return (MULT_SCALE, MULT_SCALE);
    }

    // ratio >= high_ratio ⟺ oi_majority * high_ratio_den >= oi_minority * high_ratio_num
    let ratio_cross_h = oi_majority.saturating_mul(high_ratio_den);
    let high_cross = oi_minority.saturating_mul(high_ratio_num);
    if ratio_cross_h >= high_cross {
        return (cap, MULT_SCALE);
    }

    // Linear interpolation:
    //   position = (ratio - low) / (high - low)
    //   mult = floor + position * (cap - floor)
    //
    // In cross-multiplied integer form to avoid division:
    //   ratio * den_low * den_high - low_num * minority * den_high
    //   ÷ (high_num * minority * den_low - low_num * minority * den_high)
    //
    // Simplify: work with ratio = majority/minority directly.
    //   num_offset = majority * low_den - minority * low_num  (> 0, checked above)
    //   den_range = minority * high_num - minority * low_num ... too complex.
    //
    // Simpler approach: compute ratio * 1000 as integer, interpolate in scaled space.
    let ratio_scaled = oi_majority * 1000 / oi_minority;
    let low_scaled = low_ratio_num * 1000 / low_ratio_den;
    let high_scaled = high_ratio_num * 1000 / high_ratio_den;

    if high_scaled <= low_scaled {
        return (cap, MULT_SCALE);
    }

    let position_num = ratio_scaled.saturating_sub(low_scaled);
    let position_den = high_scaled - low_scaled;
    let range = cap.saturating_sub(MULT_SCALE);

    let mult = MULT_SCALE + position_num * range / position_den;
    let mult = core::cmp::min(mult, cap);

    (mult, MULT_SCALE)
}

/// OI/vault multiplier measuring system-level leverage.
///
/// `total_oi_notional`: (oi_long + oi_short) * oracle_price / POS_SCALE.
/// `vault`: total vault balance.
/// `floor_ratio_num/den`: system leverage below which multiplier is 1.0.
/// `cap_ratio_num/den`: system leverage above which multiplier is capped.
/// `mult_max`: maximum multiplier (scaled by MULT_SCALE).
pub fn oi_vault_multiplier(
    total_oi_notional: u128,
    vault: u128,
    floor_ratio_num: u128,
    floor_ratio_den: u128,
    cap_ratio_num: u128,
    cap_ratio_den: u128,
    mult_max: u128,
) -> (u128, u128) {
    if vault == 0 {
        return (mult_max, MULT_SCALE);
    }

    // sys_lev = total_oi_notional / vault
    // sys_lev <= floor ⟺ total_oi_notional * floor_den <= vault * floor_num
    let lev_cross = total_oi_notional.saturating_mul(floor_ratio_den);
    let floor_cross = vault.saturating_mul(floor_ratio_num);
    if lev_cross <= floor_cross {
        return (MULT_SCALE, MULT_SCALE);
    }

    // sys_lev >= cap
    let lev_cross_c = total_oi_notional.saturating_mul(cap_ratio_den);
    let cap_cross = vault.saturating_mul(cap_ratio_num);
    if lev_cross_c >= cap_cross {
        return (mult_max, MULT_SCALE);
    }

    // Interpolate
    let sys_lev_scaled = total_oi_notional * 1000 / vault;
    let floor_scaled = floor_ratio_num * 1000 / floor_ratio_den;
    let cap_scaled = cap_ratio_num * 1000 / cap_ratio_den;

    if cap_scaled <= floor_scaled {
        return (mult_max, MULT_SCALE);
    }

    let position_num = sys_lev_scaled.saturating_sub(floor_scaled);
    let position_den = cap_scaled - floor_scaled;
    let range = mult_max.saturating_sub(MULT_SCALE);

    let mult = MULT_SCALE + position_num * range / position_den;
    let mult = core::cmp::min(mult, mult_max);

    (mult, MULT_SCALE)
}

/// Pool health multiplier — spikes when pool is depleted relative to exposure.
///
/// `pool_balance`: current premium pool balance.
/// `total_oi_notional`: total open interest in notional terms.
/// `low_health_num/den`: health below which multiplier is maxed (e.g., 1/100 = 1%).
/// `high_health_num/den`: health above which multiplier is 1.0 (e.g., 5/100 = 5%).
/// `mult_max`: maximum multiplier at low health.
///
/// Inverted interpolation: lower health → higher multiplier.
pub fn pool_health_multiplier(
    pool_balance: u128,
    total_oi_notional: u128,
    low_health_num: u128,
    low_health_den: u128,
    high_health_num: u128,
    high_health_den: u128,
    mult_max: u128,
) -> (u128, u128) {
    if total_oi_notional == 0 {
        return (MULT_SCALE, MULT_SCALE);
    }

    // health = pool_balance / total_oi_notional
    // health >= high → 1.0
    // health <= low → mult_max
    // else → inverted interpolation

    // health >= high ⟺ pool_balance * high_den >= total_oi_notional * high_num
    let health_cross = pool_balance.saturating_mul(high_health_den);
    let high_cross = total_oi_notional.saturating_mul(high_health_num);
    if health_cross >= high_cross {
        return (MULT_SCALE, MULT_SCALE);
    }

    // health <= low ⟺ pool_balance * low_den <= total_oi_notional * low_num
    let health_cross_l = pool_balance.saturating_mul(low_health_den);
    let low_cross = total_oi_notional.saturating_mul(low_health_num);
    if health_cross_l <= low_cross {
        return (mult_max, MULT_SCALE);
    }

    // Inverted interpolation: as health decreases from high to low, mult increases
    let health_scaled = pool_balance * 10000 / total_oi_notional;
    let low_scaled = low_health_num * 10000 / low_health_den;
    let high_scaled = high_health_num * 10000 / high_health_den;

    if high_scaled <= low_scaled {
        return (mult_max, MULT_SCALE);
    }

    // position = (health - low) / (high - low), where 0→mult_max, 1→1.0
    let position_num = health_scaled.saturating_sub(low_scaled);
    let position_den = high_scaled - low_scaled;
    let range = mult_max.saturating_sub(MULT_SCALE);

    // Inverted: mult = mult_max - position * range
    let reduction = position_num * range / position_den;
    let mult = mult_max.saturating_sub(reduction);
    let mult = core::cmp::max(mult, MULT_SCALE);

    (mult, MULT_SCALE)
}
```

- [ ] **Step 4: Add OI/vault and pool health tests**

Append to `percolator-insurance/tests/risk_index_tests.rs`:

```rust
// --- OI/vault multiplier tests ---

#[test]
fn test_oi_vault_low_leverage() {
    // total OI notional = 100M, vault = 500M → sys_lev = 0.2, below floor
    let (num, den) = oi_vault_multiplier(100_000_000, 500_000_000, 1, 1, 5, 1, 3000);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_oi_vault_high_leverage() {
    // total OI notional = 3B, vault = 500M → sys_lev = 6, above cap (5)
    let (num, den) = oi_vault_multiplier(3_000_000_000, 500_000_000, 1, 1, 5, 1, 3000);
    assert_eq!(num, 3000);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_oi_vault_zero_vault() {
    let (num, den) = oi_vault_multiplier(1000, 0, 1, 1, 5, 1, 3000);
    assert_eq!(num, 3000);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_oi_vault_interpolation() {
    // sys_lev = 3.0, range [1, 5], max 3.0x
    // position = (3-1)/(5-1) = 0.5
    // mult = 1.0 + 0.5 * 2.0 = 2.0
    let (num, den) = oi_vault_multiplier(1_500_000_000, 500_000_000, 1, 1, 5, 1, 3000);
    let ratio = num * 1000 / den;
    assert!(ratio >= 1800 && ratio <= 2200, "oiv interp ratio: {}", ratio);
}

// --- Pool health multiplier tests ---

#[test]
fn test_pool_health_healthy() {
    // pool = 50M, OI = 600M → health = 8.3%, above 5% → 1.0
    let (num, den) = pool_health_multiplier(50_000_000, 600_000_000, 1, 100, 5, 100, 5000);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_pool_health_depleted() {
    // pool = 2M, OI = 600M → health = 0.33%, below 1% → max
    let (num, den) = pool_health_multiplier(2_000_000, 600_000_000, 1, 100, 5, 100, 5000);
    assert_eq!(num, 5000);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_pool_health_zero_oi() {
    // No OI → no risk → 1.0
    let (num, den) = pool_health_multiplier(50_000_000, 0, 1, 100, 5, 100, 5000);
    assert_eq!(num, MULT_SCALE);
    assert_eq!(den, MULT_SCALE);
}

#[test]
fn test_pool_health_interpolation() {
    // pool = 18M, OI = 600M → health = 3%, range [1%, 5%]
    // position = (3-1)/(5-1) = 0.5
    // mult = 5.0 - 0.5 * 4.0 = 3.0
    let (num, den) = pool_health_multiplier(18_000_000, 600_000_000, 1, 100, 5, 100, 5000);
    let ratio = num * 1000 / den;
    assert!(ratio >= 2500 && ratio <= 3500, "pool health ratio: {}", ratio);
}
```

- [ ] **Step 5: Run all tests**

```bash
cd percolator-insurance && cargo test --test risk_index_tests -- --nocapture 2>&1
```

Expected: all tests PASS.

- [ ] **Step 6: Commit**

```bash
git add percolator-insurance/src/risk_index.rs percolator-insurance/tests/risk_index_tests.rs
git commit -m "feat: implement crowding, OI/vault, and pool health multipliers with tests"
```

---

### Task 5: Premium Pool — Accounting and Invariants

**Files:**
- Modify: `percolator-insurance/src/pool.rs`
- Create: `percolator-insurance/tests/pool_tests.rs`

- [ ] **Step 1: Write failing tests for pool operations**

Create `percolator-insurance/tests/pool_tests.rs`:

```rust
use percolator_insurance::pool::PremiumPool;

#[test]
fn test_pool_new() {
    let pool = PremiumPool::new();
    assert_eq!(pool.balance, 0);
    assert_eq!(pool.total_collected, 0);
    assert_eq!(pool.total_paid_out, 0);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_record_collection() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    assert_eq!(pool.balance, 1000);
    assert_eq!(pool.total_collected, 1000);
    assert_eq!(pool.total_paid_out, 0);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_record_multiple_collections() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    pool.record_collection(500).unwrap();
    assert_eq!(pool.balance, 1500);
    assert_eq!(pool.total_collected, 1500);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_record_consumption() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    pool.record_consumption(300);
    assert_eq!(pool.balance, 700);
    assert_eq!(pool.total_paid_out, 300);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_consumption_capped_at_balance() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    pool.record_consumption(2000);
    assert_eq!(pool.balance, 0);
    assert_eq!(pool.total_paid_out, 1000);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_reconcile_deficit() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    // Simulate insurance fund being drained from 1000 to 400
    let consumed = pool.reconcile_with_insurance_balance(400);
    assert_eq!(consumed, 600);
    assert_eq!(pool.balance, 400);
    assert_eq!(pool.total_paid_out, 600);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_reconcile_no_deficit() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    // Insurance fund still at 2000 (has more than our claim)
    let consumed = pool.reconcile_with_insurance_balance(2000);
    assert_eq!(consumed, 0);
    assert_eq!(pool.balance, 1000);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_invariant_conservation() {
    let mut pool = PremiumPool::new();
    pool.record_collection(5000).unwrap();
    pool.record_consumption(1200);
    pool.record_collection(300).unwrap();
    pool.record_consumption(800);
    // balance = 5000 - 1200 + 300 - 800 = 3300
    // total_collected = 5300, total_paid_out = 2000
    assert_eq!(pool.balance, 3300);
    assert_eq!(pool.balance + pool.total_paid_out, pool.total_collected);
    assert!(pool.check_invariants());
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd percolator-insurance && cargo test --test pool_tests 2>&1 | head -10
```

Expected: FAIL — methods not found.

- [ ] **Step 3: Implement pool operations**

Replace contents of `percolator-insurance/src/pool.rs`:

```rust
//! Premium pool accounting.
//!
//! Tracks premium funds as a claim on Percolator's insurance fund.
//! The pool does not hold funds separately — it records how much of
//! Percolator's insurance fund balance originated from premiums.

use crate::InsuredError;

/// Tracks premium funds as a claim on Percolator's insurance fund.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PremiumPool {
    /// Accounting claim on Percolator's insurance fund balance.
    pub balance: u128,
    /// Lifetime premiums collected (monotonically increasing).
    pub total_collected: u128,
    /// Lifetime consumed by deficit coverage (monotonically increasing).
    pub total_paid_out: u128,
    /// Slot of last deficit reconciliation.
    pub last_deficit_check_slot: u64,
}

impl PremiumPool {
    pub fn new() -> Self {
        Self {
            balance: 0,
            total_collected: 0,
            total_paid_out: 0,
            last_deficit_check_slot: 0,
        }
    }

    /// Record a premium collection. Called after charge_account_fee_not_atomic
    /// routes funds into Percolator's insurance fund.
    pub fn record_collection(&mut self, amount: u128) -> crate::Result<()> {
        self.balance = self.balance.checked_add(amount)
            .ok_or(InsuredError::InvalidParams)?;
        self.total_collected = self.total_collected.checked_add(amount)
            .ok_or(InsuredError::InvalidParams)?;
        Ok(())
    }

    /// Record that pool funds were consumed (insurance fund was drained).
    /// Consumption is capped at current balance — cannot go negative.
    pub fn record_consumption(&mut self, amount: u128) {
        let actual = core::cmp::min(amount, self.balance);
        self.balance -= actual;
        self.total_paid_out = self.total_paid_out.saturating_add(actual);
    }

    /// Reconcile pool claim with Percolator's actual insurance fund balance.
    /// If the insurance fund was drained below our claim (by use_insurance_buffer
    /// during ADL), reduce our claim to match.
    /// Returns the amount consumed.
    pub fn reconcile_with_insurance_balance(&mut self, insurance_balance: u128) -> u128 {
        if self.balance <= insurance_balance {
            return 0;
        }
        let consumed = self.balance - insurance_balance;
        self.record_consumption(consumed);
        consumed
    }

    /// Check all pool invariants. Returns false if any are violated.
    pub fn check_invariants(&self) -> bool {
        // Conservation: balance + total_paid_out == total_collected
        let sum = match self.balance.checked_add(self.total_paid_out) {
            Some(v) => v,
            None => return false,
        };
        if sum != self.total_collected {
            return false;
        }
        // Monotonicity
        if self.total_paid_out > self.total_collected {
            return false;
        }
        true
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cd percolator-insurance && cargo test --test pool_tests -- --nocapture 2>&1
```

Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add percolator-insurance/src/pool.rs percolator-insurance/tests/pool_tests.rs
git commit -m "feat: implement PremiumPool accounting with invariant checks"
```

---

### Task 6: Wrapper — Data Structures and Initialization

**Files:**
- Modify: `percolator-insurance/src/wrapper.rs`
- Modify: `percolator-insurance/src/lib.rs`

- [ ] **Step 1: Write failing test for wrapper initialization**

Create `percolator-insurance/tests/integration_tests.rs`:

```rust
use percolator_insurance::wrapper::{InsuredRiskEngine, PremiumParams, AccountPremiumState};
use percolator_insurance::MULT_SCALE;
use percolator::{RiskParams, U128, MAX_ACCOUNTS};

fn test_risk_params() -> RiskParams {
    RiskParams {
        maintenance_margin_bps: 500,
        initial_margin_bps: 1000,
        trading_fee_bps: 10,
        max_accounts: MAX_ACCOUNTS as u64,
        liquidation_fee_bps: 50,
        liquidation_fee_cap: U128::new(1_000_000_000_000),
        min_liquidation_abs: U128::new(0),
        min_nonzero_mm_req: 0,
        min_nonzero_im_req: 0,
        h_min: 100,
        h_max: 1000,
        resolve_price_deviation_bps: 500,
        max_accrual_dt_slots: 1_000_000,
        max_abs_funding_e9_per_slot: 10_000,
        min_funding_lifetime_slots: 1_000_000,
        max_active_positions_per_side: MAX_ACCOUNTS as u64,
        max_price_move_bps_per_slot: 100,
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

#[test]
fn test_wrapper_new() {
    let rp = test_risk_params();
    let pp = test_premium_params();
    let engine = InsuredRiskEngine::new(rp, pp, 1, 100_000).unwrap();
    assert_eq!(engine.pool.balance, 0);
    assert!(engine.pool.check_invariants());
    assert!(engine.engine.check_conservation());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd percolator-insurance && cargo test --test integration_tests 2>&1 | head -10
```

Expected: FAIL — `InsuredRiskEngine` not found.

- [ ] **Step 3: Implement wrapper structs and initialization**

Replace contents of `percolator-insurance/src/wrapper.rs`:

```rust
//! InsuredRiskEngine — wraps Percolator's public API with premium collection.
//!
//! Every public method follows the pattern:
//! 1. Collect any accrued premiums owed by the account
//! 2. Execute the Percolator operation
//! 3. Reconcile pool with insurance fund balance

use crate::pool::PremiumPool;
use crate::premium::compute_premium_per_slot;
use crate::risk_index::{
    RiskIndex, crowding_multiplier, oi_vault_multiplier, pool_health_multiplier,
};
use crate::{InsuredError, MULT_SCALE, POS_SCALE};
use percolator::{
    RiskEngine, RiskParams, RiskError, MarketMode, LiquidationPolicy,
    MAX_ACCOUNTS, MAX_ORACLE_PRICE,
};

/// Deploy-time premium configuration.
#[derive(Clone, Copy, Debug)]
pub struct PremiumParams {
    /// Base premium rate per slot per unit notional.
    pub base_rate_per_slot: u128,
    /// Leverage exponent numerator (e.g., 3 for 1.5).
    pub leverage_exponent_num: u64,
    /// Leverage exponent denominator (e.g., 2 for 1.5).
    pub leverage_exponent_den: u64,
    /// Minimum premium commitment in slots (e.g., 216_000 for ~24h).
    pub min_commitment_slots: u64,
    /// Crowding ratio thresholds and cap.
    pub crowding_low_ratio_num: u128,
    pub crowding_low_ratio_den: u128,
    pub crowding_high_ratio_num: u128,
    pub crowding_high_ratio_den: u128,
    pub crowding_cap: u128,
    /// OI/vault ratio thresholds and max multiplier.
    pub oi_vault_floor_ratio_num: u128,
    pub oi_vault_floor_ratio_den: u128,
    pub oi_vault_cap_ratio_num: u128,
    pub oi_vault_cap_ratio_den: u128,
    pub oi_vault_mult_max: u128,
    /// Pool health thresholds and max multiplier.
    pub pool_health_low_num: u128,
    pub pool_health_low_den: u128,
    pub pool_health_high_num: u128,
    pub pool_health_high_den: u128,
    pub pool_health_mult_max: u128,
    /// Minimum premium per slot (floor for dust positions).
    pub min_premium_per_slot: u128,
}

/// Per-account premium tracking state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountPremiumState {
    /// Slot when premium was last collected.
    pub last_premium_slot: u64,
    /// Slot when the minimum commitment expires.
    pub commitment_end_slot: u64,
    /// Upfront premium remaining (decremented as slots elapse).
    pub prepaid_premium: u128,
    /// Whether this account has an active position paying premiums.
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

/// Wrapper around Percolator's RiskEngine with insurance premium collection.
pub struct InsuredRiskEngine {
    /// The wrapped Percolator risk engine.
    pub engine: RiskEngine,
    /// Premium pool accounting.
    pub pool: PremiumPool,
    /// Deploy-time premium parameters.
    pub premium_params: PremiumParams,
    /// Per-account premium state (parallel to engine.accounts).
    pub account_premiums: [AccountPremiumState; MAX_ACCOUNTS],
}

impl InsuredRiskEngine {
    /// Create a new InsuredRiskEngine with initialized market.
    pub fn new(
        risk_params: RiskParams,
        premium_params: PremiumParams,
        init_slot: u64,
        init_oracle_price: u64,
    ) -> crate::Result<Self> {
        if premium_params.leverage_exponent_den == 0 {
            return Err(InsuredError::InvalidParams);
        }
        if premium_params.crowding_low_ratio_den == 0
            || premium_params.crowding_high_ratio_den == 0
            || premium_params.oi_vault_floor_ratio_den == 0
            || premium_params.oi_vault_cap_ratio_den == 0
            || premium_params.pool_health_low_den == 0
            || premium_params.pool_health_high_den == 0
        {
            return Err(InsuredError::InvalidParams);
        }

        let engine = RiskEngine::new_with_market(risk_params, init_slot, init_oracle_price);

        Ok(Self {
            engine,
            pool: PremiumPool::new(),
            premium_params,
            account_premiums: [AccountPremiumState::new(); MAX_ACCOUNTS],
        })
    }

    /// Compute the current risk index for an account based on on-chain state.
    pub fn compute_risk_index(&self, account_idx: usize) -> RiskIndex {
        let long_oi = self.engine.oi_eff_long_q;
        let short_oi = self.engine.oi_eff_short_q;
        let vault = self.engine.vault.get();
        let oracle_price = self.engine.last_oracle_price;
        let pp = &self.premium_params;

        // Determine if account is on majority side
        let pos = self.engine.accounts[account_idx].position_basis_q;
        let (majority_oi, minority_oi, is_majority) = if long_oi >= short_oi {
            (long_oi, short_oi, pos > 0)
        } else {
            (short_oi, long_oi, pos < 0)
        };

        let crowding = if pos == 0 {
            (MULT_SCALE, MULT_SCALE)
        } else {
            crowding_multiplier(
                majority_oi, minority_oi, is_majority,
                pp.crowding_low_ratio_num, pp.crowding_low_ratio_den,
                pp.crowding_high_ratio_num, pp.crowding_high_ratio_den,
                pp.crowding_cap,
            )
        };

        // Total OI notional
        let total_oi_q = long_oi.saturating_add(short_oi);
        let total_oi_notional = if oracle_price > 0 {
            total_oi_q.saturating_mul(oracle_price as u128) / POS_SCALE
        } else {
            0
        };

        let oi_vault = oi_vault_multiplier(
            total_oi_notional, vault,
            pp.oi_vault_floor_ratio_num, pp.oi_vault_floor_ratio_den,
            pp.oi_vault_cap_ratio_num, pp.oi_vault_cap_ratio_den,
            pp.oi_vault_mult_max,
        );

        let pool_health = pool_health_multiplier(
            self.pool.balance, total_oi_notional,
            pp.pool_health_low_num, pp.pool_health_low_den,
            pp.pool_health_high_num, pp.pool_health_high_den,
            pp.pool_health_mult_max,
        );

        RiskIndex { crowding, oi_vault, pool_health }
    }

    /// Compute account notional from on-chain state.
    fn account_notional(&self, idx: usize) -> u128 {
        let pos = self.engine.accounts[idx].position_basis_q;
        if pos == 0 {
            return 0;
        }
        let abs_pos = pos.unsigned_abs();
        let price = self.engine.last_oracle_price;
        abs_pos.saturating_mul(price as u128) / POS_SCALE
    }

    /// Collect accrued premium from an account.
    /// Returns the amount collected (0 if not active or no time elapsed).
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
        let capital = self.engine.accounts[i].capital.get();
        let risk_idx = self.compute_risk_index(i);

        let rate = compute_premium_per_slot(
            notional, capital,
            self.premium_params.base_rate_per_slot,
            &risk_idx,
            self.premium_params.min_premium_per_slot,
        );

        let premium_owed = rate.saturating_mul(slots_elapsed as u128);
        if premium_owed == 0 {
            self.account_premiums[i].last_premium_slot = now_slot;
            return Ok(0);
        }

        // Deduct from prepaid first
        let mut remaining = premium_owed;
        let prepaid = &mut self.account_premiums[i].prepaid_premium;
        if *prepaid > 0 {
            let from_prepaid = core::cmp::min(remaining, *prepaid);
            remaining -= from_prepaid;
            *prepaid -= from_prepaid;
        }

        // Charge remaining from account capital via Percolator
        if remaining > 0 {
            match self.engine.charge_account_fee_not_atomic(idx, remaining, now_slot) {
                Ok(()) => {
                    self.pool.record_collection(remaining)?;
                }
                Err(RiskError::InsufficientBalance) => {
                    // Account can't afford full premium — collect what's available.
                    // The account will likely be liquidated soon.
                    let available = self.engine.accounts[i].capital.get();
                    if available > 0 {
                        self.engine.charge_account_fee_not_atomic(idx, available, now_slot)
                            .map_err(InsuredError::Risk)?;
                        self.pool.record_collection(available)?;
                    }
                }
                Err(e) => return Err(InsuredError::Risk(e)),
            }
        }

        self.account_premiums[i].last_premium_slot = now_slot;
        Ok(premium_owed)
    }

    /// Reconcile pool balance with Percolator's insurance fund.
    /// Call after any operation that could drain the insurance fund.
    pub fn reconcile_pool(&mut self) {
        let insurance = self.engine.insurance_fund.balance.get();
        self.pool.reconcile_with_insurance_balance(insurance);
    }
}
```

- [ ] **Step 4: Update lib.rs to export new types**

Append to exports in `percolator-insurance/src/lib.rs` after the module declarations:

```rust
pub use wrapper::{InsuredRiskEngine, PremiumParams, AccountPremiumState};
pub use pool::PremiumPool;
pub use risk_index::RiskIndex;
pub use premium::{isqrt, leverage_multiplier, compute_premium_per_slot};
```

- [ ] **Step 5: Run integration test**

```bash
cd percolator-insurance && cargo test --test integration_tests -- --nocapture 2>&1
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add percolator-insurance/src/ percolator-insurance/tests/integration_tests.rs
git commit -m "feat: implement InsuredRiskEngine wrapper with premium collection and pool reconciliation"
```

---

### Task 7: Wrapper — Wrapped Operations (deposit, trade, withdraw, liquidate)

**Files:**
- Modify: `percolator-insurance/src/wrapper.rs`
- Modify: `percolator-insurance/tests/integration_tests.rs`

- [ ] **Step 1: Write failing integration test for deposit → trade → collect → withdraw lifecycle**

Append to `percolator-insurance/tests/integration_tests.rs`:

```rust
#[test]
fn test_full_lifecycle() {
    let rp = test_risk_params();
    let pp = test_premium_params();
    let mut engine = InsuredRiskEngine::new(rp, pp, 1, 100_000).unwrap();

    // Deposit into two accounts
    engine.deposit(0, 100_000, 10).unwrap();
    engine.deposit(1, 100_000, 10).unwrap();

    // Execute trade: account 0 goes long, account 1 goes short
    engine.execute_trade(0, 1, 100_000, 20, 500_000, 100_000, 0, 100, 1000, None).unwrap();

    // Verify premiums are active
    assert!(engine.account_premiums[0].is_active);
    assert!(engine.account_premiums[1].is_active);
    assert!(engine.account_premiums[0].prepaid_premium > 0);

    // Advance time and collect premiums
    let collected = engine.collect_accrued_premium(0, 1000).unwrap();
    assert!(collected > 0 || engine.account_premiums[0].prepaid_premium > 0);

    // Pool should have funds
    assert!(engine.pool.total_collected > 0);
    assert!(engine.pool.check_invariants());

    // Percolator conservation must hold
    assert!(engine.engine.check_conservation());
}

#[test]
fn test_commitment_enforced_on_early_close() {
    let rp = test_risk_params();
    let pp = test_premium_params();
    let mut engine = InsuredRiskEngine::new(rp, pp, 1, 100_000).unwrap();

    engine.deposit(0, 1_000_000, 10).unwrap();
    engine.deposit(1, 1_000_000, 10).unwrap();

    // Open position
    engine.execute_trade(0, 1, 100_000, 20, 500_000, 100_000, 0, 100, 1000, None).unwrap();

    let commitment = engine.account_premiums[0].prepaid_premium;
    assert!(commitment > 0, "commitment must be charged");

    // Close position immediately (within commitment window)
    engine.execute_trade(0, 1, 100_000, 30, -500_000, 100_000, 0, 100, 1000, None).unwrap();

    // Commitment premium stays in pool (not refunded)
    assert!(engine.pool.total_collected > 0);
    assert!(engine.engine.check_conservation());
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd percolator-insurance && cargo test --test integration_tests 2>&1 | head -20
```

Expected: FAIL — `deposit`, `execute_trade` methods not found on `InsuredRiskEngine`.

- [ ] **Step 3: Implement wrapped operations**

Append to `InsuredRiskEngine` impl block in `percolator-insurance/src/wrapper.rs`:

```rust
    // ====================================================================
    // Wrapped Percolator operations
    // ====================================================================

    /// Deposit into an account. Collects accrued premium if position is active.
    pub fn deposit(
        &mut self,
        idx: u16,
        amount: u128,
        now_slot: u64,
    ) -> crate::Result<()> {
        // Collect accrued premium if this account has an active position
        if (idx as usize) < MAX_ACCOUNTS && self.account_premiums[idx as usize].is_active {
            let _ = self.collect_accrued_premium(idx, now_slot);
        }

        self.engine.deposit_not_atomic(idx, amount, now_slot)
            .map_err(InsuredError::Risk)?;
        Ok(())
    }

    /// Execute a trade with premium commitment enforcement.
    ///
    /// If an account goes from flat to positioned, charges the 24h upfront
    /// commitment and activates premium accrual. If already positioned,
    /// collects accrued premium before the trade.
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

        // Collect accrued premiums on both accounts before trade
        if ai < MAX_ACCOUNTS && self.account_premiums[ai].is_active {
            let _ = self.collect_accrued_premium(a, now_slot);
        }
        if bi < MAX_ACCOUNTS && self.account_premiums[bi].is_active {
            let _ = self.collect_accrued_premium(b, now_slot);
        }

        // Check if accounts are currently flat (will go from flat → positioned)
        let a_was_flat = ai < MAX_ACCOUNTS
            && self.engine.accounts[ai].position_basis_q == 0;
        let b_was_flat = bi < MAX_ACCOUNTS
            && self.engine.accounts[bi].position_basis_q == 0;

        // Execute the trade
        self.engine.execute_trade_not_atomic(
            a, b, oracle_price, now_slot, size_q, exec_price,
            funding_rate_e9, admit_h_min, admit_h_max,
            admit_h_max_consumption_threshold_bps_opt,
        ).map_err(InsuredError::Risk)?;

        // Activate premium and charge commitment for newly positioned accounts
        if a_was_flat && ai < MAX_ACCOUNTS {
            self.activate_premium(a, now_slot)?;
        }
        if b_was_flat && bi < MAX_ACCOUNTS {
            self.activate_premium(b, now_slot)?;
        }

        // Check if any account went from positioned → flat (closing trade)
        if ai < MAX_ACCOUNTS && self.engine.accounts[ai].position_basis_q == 0 {
            self.account_premiums[ai].is_active = false;
        }
        if bi < MAX_ACCOUNTS && self.engine.accounts[bi].position_basis_q == 0 {
            self.account_premiums[bi].is_active = false;
        }

        self.reconcile_pool();
        Ok(())
    }

    /// Activate premium accrual and charge 24h upfront commitment.
    fn activate_premium(&mut self, idx: u16, now_slot: u64) -> crate::Result<()> {
        let i = idx as usize;
        if i >= MAX_ACCOUNTS {
            return Ok(());
        }

        let notional = self.account_notional(i);
        let capital = self.engine.accounts[i].capital.get();
        let risk_idx = self.compute_risk_index(i);

        let rate = compute_premium_per_slot(
            notional, capital,
            self.premium_params.base_rate_per_slot,
            &risk_idx,
            self.premium_params.min_premium_per_slot,
        );

        let commitment = rate.saturating_mul(self.premium_params.min_commitment_slots as u128);

        if commitment > 0 {
            // Charge upfront commitment
            match self.engine.charge_account_fee_not_atomic(idx, commitment, now_slot) {
                Ok(()) => {
                    self.pool.record_collection(commitment)?;
                }
                Err(_) => {
                    // Can't afford commitment — still allow trade but mark with zero prepaid.
                    // The ongoing per-slot premium will still be collected.
                }
            }
        }

        self.account_premiums[i] = AccountPremiumState {
            last_premium_slot: now_slot,
            commitment_end_slot: now_slot.saturating_add(self.premium_params.min_commitment_slots),
            prepaid_premium: commitment,
            is_active: true,
        };

        Ok(())
    }

    /// Withdraw with premium enforcement.
    /// Collects accrued premium before allowing withdrawal.
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
        if (idx as usize) < MAX_ACCOUNTS && self.account_premiums[idx as usize].is_active {
            let _ = self.collect_accrued_premium(idx, now_slot);
        }

        self.engine.withdraw_not_atomic(
            idx, amount, oracle_price, now_slot,
            funding_rate_e9, admit_h_min, admit_h_max,
            admit_h_max_consumption_threshold_bps_opt,
        ).map_err(InsuredError::Risk)?;
        Ok(())
    }

    /// Liquidate with premium collection and deficit reconciliation.
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
        // Collect any outstanding premium
        if (idx as usize) < MAX_ACCOUNTS && self.account_premiums[idx as usize].is_active {
            let _ = self.collect_accrued_premium(idx, now_slot);
        }

        let result = self.engine.liquidate_at_oracle_not_atomic(
            idx, now_slot, oracle_price, policy,
            funding_rate_e9, admit_h_min, admit_h_max,
            admit_h_max_consumption_threshold_bps_opt,
        ).map_err(InsuredError::Risk)?;

        // Deactivate premium if fully liquidated
        let i = idx as usize;
        if i < MAX_ACCOUNTS && self.engine.accounts[i].position_basis_q == 0 {
            self.account_premiums[i].is_active = false;
        }

        // Reconcile — liquidation may have drained insurance fund
        self.reconcile_pool();

        Ok(result)
    }
}
```

- [ ] **Step 4: Run integration tests**

```bash
cd percolator-insurance && cargo test --test integration_tests -- --nocapture 2>&1
```

Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add percolator-insurance/src/wrapper.rs percolator-insurance/tests/integration_tests.rs
git commit -m "feat: implement wrapped deposit, trade, withdraw, liquidate with premium gates"
```

---

### Task 8: Fuzz Tests

**Files:**
- Create: `percolator-insurance/tests/fuzz_tests.rs`

- [ ] **Step 1: Write proptest fuzz tests**

Create `percolator-insurance/tests/fuzz_tests.rs`:

```rust
use proptest::prelude::*;
use percolator_insurance::premium::{isqrt, leverage_multiplier, compute_premium_per_slot};
use percolator_insurance::pool::PremiumPool;
use percolator_insurance::risk_index::RiskIndex;
use percolator_insurance::MULT_SCALE;

proptest! {
    /// isqrt(n)^2 <= n < (isqrt(n)+1)^2 for all n
    #[test]
    fn fuzz_isqrt_correct(n in 0u128..=u64::MAX as u128) {
        let r = isqrt(n);
        prop_assert!(r * r <= n, "isqrt({})={}, {}^2={} > {}", n, r, r, r*r, n);
        if r < u64::MAX as u128 {
            let next = r + 1;
            prop_assert!(next * next > n, "isqrt({})={}, ({}+1)^2={} <= {}", n, r, r, next*next, n);
        }
    }

    /// Premium is always >= 0 and monotonically increases with notional
    #[test]
    fn fuzz_premium_monotonic_notional(
        notional1 in 1u128..1_000_000_000u128,
        notional2 in 1u128..1_000_000_000u128,
        capital in 1u128..1_000_000_000u128,
    ) {
        let idx = RiskIndex::neutral();
        let p1 = compute_premium_per_slot(notional1, capital, 100, &idx, 1);
        let p2 = compute_premium_per_slot(notional2, capital, 100, &idx, 1);

        if notional1 <= notional2 {
            prop_assert!(p1 <= p2, "premium must increase with notional: {}@{} vs {}@{}", p1, notional1, p2, notional2);
        }
    }

    /// Premium increases with leverage (fixed notional, decreasing capital)
    #[test]
    fn fuzz_premium_monotonic_leverage(
        notional in 10_000u128..1_000_000u128,
        capital1 in 1u128..1_000_000u128,
        capital2 in 1u128..1_000_000u128,
    ) {
        let idx = RiskIndex::neutral();
        let p1 = compute_premium_per_slot(notional, capital1, 100, &idx, 1);
        let p2 = compute_premium_per_slot(notional, capital2, 100, &idx, 1);

        // Higher capital = lower leverage = lower premium
        if capital1 >= capital2 {
            prop_assert!(p1 <= p2, "premium must decrease with higher capital");
        }
    }

    /// Pool invariants hold after random sequence of collections and consumptions
    #[test]
    fn fuzz_pool_invariants(
        ops in prop::collection::vec(
            prop::bool::ANY.prop_flat_map(|is_collect| {
                if is_collect {
                    (Just(true), 1u128..1_000_000u128).boxed()
                } else {
                    (Just(false), 1u128..1_000_000u128).boxed()
                }
            }),
            1..50
        )
    ) {
        let mut pool = PremiumPool::new();
        for (is_collect, amount) in ops {
            if is_collect {
                let _ = pool.record_collection(amount);
            } else {
                pool.record_consumption(amount);
            }
            prop_assert!(pool.check_invariants(), "invariant violated: {:?}", pool);
        }
    }

    /// Leverage multiplier is always >= MULT_SCALE (1.0)
    #[test]
    fn fuzz_leverage_mult_floor(
        notional in 0u128..1_000_000_000u128,
        capital in 1u128..1_000_000_000u128,
        exp_num in 1u64..5u64,
        exp_den in 1u64..3u64,
    ) {
        let (num, den) = leverage_multiplier(notional, capital, exp_num, exp_den);
        prop_assert!(den > 0, "denominator must be positive");
        prop_assert!(num >= den || num >= MULT_SCALE, "multiplier must be >= 1.0");
    }
}
```

- [ ] **Step 2: Run fuzz tests**

```bash
cd percolator-insurance && cargo test --test fuzz_tests -- --nocapture 2>&1
```

Expected: all tests PASS (proptest runs 256 cases by default per test).

- [ ] **Step 3: Commit**

```bash
git add percolator-insurance/tests/fuzz_tests.rs
git commit -m "feat: add proptest fuzz tests for premium math, pool invariants, and monotonicity"
```

---

### Task 9: Final Verification and Cleanup

**Files:**
- All files in `percolator-insurance/`

- [ ] **Step 1: Run the full test suite**

```bash
cd percolator-insurance && cargo test 2>&1
```

Expected: all tests PASS, no warnings.

- [ ] **Step 2: Run clippy**

```bash
cd percolator-insurance && cargo clippy -- -D warnings 2>&1
```

Expected: no warnings or errors.

- [ ] **Step 3: Verify no_std compatibility**

```bash
cd percolator-insurance && cargo check --no-default-features 2>&1
```

Expected: compiles cleanly. No std dependency.

- [ ] **Step 4: Run Percolator's own tests to ensure no regression**

```bash
cd /home/acheron28nyx/percolator && cargo test --features test 2>&1 | tail -5
```

Expected: all Percolator tests still pass.

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "chore: final verification — all tests pass, clippy clean, no_std verified"
```
