//! Kani formal-verification proof harnesses.
//!
//! This module is compiled ONLY under `cargo kani` (it is gated behind
//! `#[cfg(kani)]` at its registration in `lib.rs`). It does NOT compile under a
//! normal `cargo build` / `cargo test` / `cargo clippy`, and therefore adds
//! zero code and zero warnings to the production build.
//!
//! Run with:
//! ```text
//! cargo kani -p percolator-insurance
//! # or a single harness:
//! cargo kani -p percolator-insurance --harness kani_pool_record_collection_preserves_invariants
//! ```
//!
//! The harnesses mirror the parent `percolator` crate's Kani style: bounded
//! `kani::any()` inputs constrained with `kani::assume(..)` to keep the proofs
//! tractable, each annotated with `#[kani::proof]`.
//!
//! What is proved here:
//! - `pool.rs`: `PremiumPool::check_invariants()` is PRESERVED across
//!   `record_collection`, `record_consumption`, and
//!   `reconcile_with_insurance_balance` for arbitrary (bounded) inputs.
//! - `premium.rs`:
//!     * `isqrt(n)` is the exact floor of √n: `x*x <= n < (x+1)*(x+1)`.
//!     * `inth_root(n, k)` is the exact floor of n^(1/k) for small k.
//!     * `compute_premium_per_slot` never panics and is monotonic
//!       non-decreasing in `notional` (and in leverage) for bounded inputs.

use crate::pool::PremiumPool;
use crate::premium::{compute_premium_per_slot, inth_root, isqrt};
use crate::risk_index::RiskIndex;

// ============================================================================
// Helpers
// ============================================================================

/// Build a valid `PremiumPool` from arbitrary, bounded fields that already
/// satisfy `check_invariants()`. We pick `total_paid_out` and `balance`
/// independently and DERIVE `total_collected = balance + total_paid_out`, which
/// makes the invariant hold by construction without an `assume` that the solver
/// would otherwise have to satisfy by search.
fn arbitrary_valid_pool() -> PremiumPool {
    let balance: u64 = kani::any();
    let total_paid_out: u64 = kani::any();

    let balance = balance as u128;
    let total_paid_out = total_paid_out as u128;
    // No overflow: both are u64-bounded, sum fits in u128.
    let total_collected = balance + total_paid_out;

    let pool = PremiumPool {
        balance,
        total_collected,
        total_paid_out,
        last_deficit_check_slot: kani::any(),
    };
    // Sanity: the constructed pool is valid.
    assert!(pool.check_invariants());
    pool
}

// ============================================================================
// pool.rs — invariant preservation
// ============================================================================

/// `record_collection` preserves `check_invariants()`.
///
/// On `Ok`, `balance` and `total_collected` both grow by `amount`, so
/// `balance + total_paid_out == total_collected` is maintained. On `Err`
/// (overflow), the pool is left unmodified, so the invariant also holds.
#[kani::proof]
fn kani_pool_record_collection_preserves_invariants() {
    let mut pool = arbitrary_valid_pool();
    let amount: u64 = kani::any();

    let _ = pool.record_collection(amount as u128);

    assert!(pool.check_invariants());
}

/// `record_consumption` preserves `check_invariants()`.
///
/// Consumption is capped at `balance`: `actual = min(amount, balance)`.
/// `balance` drops by `actual` and `total_paid_out` rises by `actual`, so the
/// sum (and thus `total_collected`) is unchanged, and `total_paid_out` cannot
/// exceed `total_collected` since `actual <= balance`.
#[kani::proof]
fn kani_pool_record_consumption_preserves_invariants() {
    let mut pool = arbitrary_valid_pool();
    let amount: u64 = kani::any();

    pool.record_consumption(amount as u128);

    assert!(pool.check_invariants());
}

/// `reconcile_with_insurance_balance` preserves `check_invariants()`.
///
/// Reconcile delegates to `record_consumption` for any shortfall
/// (`balance - insurance_balance`) and otherwise leaves the pool untouched, so
/// the invariant is preserved on both branches.
#[kani::proof]
fn kani_pool_reconcile_preserves_invariants() {
    let mut pool = arbitrary_valid_pool();
    let insurance_balance: u64 = kani::any();

    let _ = pool.reconcile_with_insurance_balance(insurance_balance as u128);

    assert!(pool.check_invariants());
}

/// The conservation identity returned by `reconcile_with_insurance_balance`:
/// the amount it reports consumed never exceeds the prior balance, and the
/// pool's accounting still balances afterwards.
#[kani::proof]
fn kani_pool_reconcile_consumed_bounded() {
    let mut pool = arbitrary_valid_pool();
    let prior_balance = pool.balance;
    let insurance_balance: u64 = kani::any();

    let consumed = pool.reconcile_with_insurance_balance(insurance_balance as u128);

    assert!(consumed <= prior_balance);
    assert!(pool.check_invariants());
}

// ============================================================================
// premium.rs — isqrt / inth_root floor correctness
// ============================================================================

/// `isqrt(n)` is exactly `floor(√n)`: `x*x <= n` and `n < (x+1)*(x+1)`.
///
/// Bound `n` to u64 so the `(x+1)*(x+1)` upper check cannot itself overflow
/// u128 (for u64-bounded `n`, `x <= 2^32` and `(x+1)^2` fits comfortably).
#[kani::proof]
fn kani_isqrt_floor_correct() {
    let n: u64 = kani::any();
    let n = n as u128;

    let x = isqrt(n);

    // Lower bound: x*x <= n (no overflow: x <= sqrt(u64::MAX) < 2^32).
    assert!(x.checked_mul(x).map(|v| v <= n).unwrap_or(false));

    // Upper bound: n < (x+1)*(x+1).
    let x1 = x + 1;
    let x1sq = x1.checked_mul(x1).unwrap();
    assert!(n < x1sq);
}

/// `isqrt` upper-bound holds at the u128 ceiling too: `floor(√(u128::MAX))`
/// satisfies `x*x <= MAX` and `(x+1)^2` overflows u128 (so there is no integer
/// above `x` whose square is `<= MAX`).
#[kani::proof]
fn kani_isqrt_handles_u128_max_region() {
    let n: u128 = kani::any();
    kani::assume(n >= u128::MAX - 3);

    let x = isqrt(n);

    // x*x <= n.
    assert!(x.checked_mul(x).map(|v| v <= n).unwrap_or(false));
    // (x+1)^2 either overflows or strictly exceeds n.
    let x1 = x + 1;
    match x1.checked_mul(x1) {
        Some(v) => assert!(v > n),
        None => {} // overflow ⇒ no representable square <= n above x.
    }
}

/// `inth_root(n, k)` is exactly `floor(n^(1/k))` for small `k`:
/// `x^k <= n` and `(x+1)^k > n`. Bound `n` to u32 and `k` to {3,4,5} so the
/// `(x+1)^k` check stays representable.
#[kani::proof]
fn kani_inth_root_floor_correct_small_k() {
    let n: u32 = kani::any();
    let n = n as u128;

    // Pick k in {3, 4, 5} — k==1 and k==2 are exercised by isqrt above.
    let sel: u8 = kani::any();
    kani::assume(sel < 3);
    let k: u32 = 3 + sel as u32;

    let x = inth_root(n, k);

    // Lower bound: x^k <= n.
    let xk = pow_checked(x, k);
    assert!(xk.map(|v| v <= n).unwrap_or(false));

    // Upper bound: (x+1)^k > n  (it may overflow u128, which also implies > n).
    let x1 = x + 1;
    match pow_checked(x1, k) {
        Some(v) => assert!(v > n),
        None => {} // (x+1)^k overflowed ⇒ certainly > n (n <= u32::MAX).
    }
}

/// Small checked integer power for the harness upper-bound checks.
fn pow_checked(base: u128, exp: u32) -> Option<u128> {
    let mut acc: u128 = 1;
    let mut i = 0;
    while i < exp {
        acc = acc.checked_mul(base)?;
        i += 1;
    }
    Some(acc)
}

// ============================================================================
// premium.rs — compute_premium_per_slot: no panic + monotonicity
// ============================================================================

/// `compute_premium_per_slot` never panics for bounded, neutral-index inputs.
///
/// Inputs are bounded to u64 to keep the proof tractable while still exercising
/// the GCD-reduction and multiplier-folding paths.
#[kani::proof]
fn kani_premium_no_panic() {
    let notional: u64 = kani::any();
    let capital: u64 = kani::any();
    let base_rate: u64 = kani::any();
    let min_premium: u64 = kani::any();

    let idx = RiskIndex::neutral();
    let _ = compute_premium_per_slot(
        notional as u128,
        capital as u128,
        base_rate as u128,
        &idx,
        min_premium as u128,
    );
    // Reaching here without a panic IS the property.
}

/// `compute_premium_per_slot` is monotonic NON-DECREASING in `notional`:
/// raising notional (with everything else fixed, neutral index) never lowers
/// the premium. Bounded to keep the leverage/GCD arithmetic in a tractable
/// range for the solver.
#[kani::proof]
fn kani_premium_monotonic_in_notional() {
    let n_lo: u32 = kani::any();
    let n_hi: u32 = kani::any();
    kani::assume(n_lo <= n_hi);

    let capital: u32 = kani::any();
    let base_rate: u32 = kani::any();

    let idx = RiskIndex::neutral();
    let p_lo = compute_premium_per_slot(
        n_lo as u128,
        capital as u128,
        base_rate as u128,
        &idx,
        0,
    );
    let p_hi = compute_premium_per_slot(
        n_hi as u128,
        capital as u128,
        base_rate as u128,
        &idx,
        0,
    );

    assert!(p_hi >= p_lo);
}

/// `compute_premium_per_slot` is monotonic NON-DECREASING in leverage: holding
/// notional, base_rate, and index fixed, a SMALLER capital (⇒ higher leverage)
/// never lowers the premium. Bounded inputs; capital is constrained `> 0` so
/// the leverage multiplier engages.
#[kani::proof]
fn kani_premium_monotonic_in_leverage() {
    let notional: u32 = kani::any();
    let base_rate: u32 = kani::any();

    let cap_lo: u32 = kani::any();
    let cap_hi: u32 = kani::any();
    kani::assume(cap_lo > 0 && cap_lo <= cap_hi);

    let idx = RiskIndex::neutral();
    // Smaller capital ⇒ higher leverage ⇒ premium should be ≥.
    let p_high_lev = compute_premium_per_slot(
        notional as u128,
        cap_lo as u128,
        base_rate as u128,
        &idx,
        0,
    );
    let p_low_lev = compute_premium_per_slot(
        notional as u128,
        cap_hi as u128,
        base_rate as u128,
        &idx,
        0,
    );

    assert!(p_high_lev >= p_low_lev);
}
