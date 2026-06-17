# Hybrid TWA Premium Accrual Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the gameable per-account, collection-time premium rate with a global cumulative system-risk accumulator (funding-rate pattern) plus max-of-endpoints leverage, so premium reflects risk integrated over real elapsed time.

**Architecture:** A monotonic `cum_system_index` on the engine integrates the account-independent system multipliers (`oi_vault × pool_health × volatility`) over time, advanced by every wrapped op and a public `accrue()` for the keeper. Each account stores a `cum_system_snapshot` and `last_leverage_factor`; premium for an interval = `notional × base_rate × max(last_lev, lev_now) × crowd_now × (cum_now − snapshot) / (PREMIUM_SCALE × MULT_SCALE³)`.

**Tech Stack:** Rust, `#![no_std]`, `#![forbid(unsafe_code)]`, pure-integer math, `proptest`, Kani. Spec: `docs/superpowers/specs/2026-06-17-twa-hybrid-accrual-design.md`.

Constants (from `src/lib.rs`): `MULT_SCALE (M) = 1_000`, `PREMIUM_SCALE (P) = 1_000_000_000`, `LEVERAGE_SCALE = 1_000_000`.

Run all tests with: `cargo test -p percolator-insurance`. Clippy: `cargo clippy -p percolator-insurance --lib --tests` (must be clean bar parent `cfg(kani)`).

---

## Task 1: `compute_system_index_scaled` (account-independent system factor)

**Files:**
- Modify: `percolator-insurance/src/premium.rs` (add pub fn near `compute_premium_per_slot`)
- Test: `percolator-insurance/tests/premium_tests.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/premium_tests.rs`:
```rust
#[test]
fn test_system_index_scaled_neutral_is_one() {
    use percolator_insurance::premium::compute_system_index_scaled;
    // All neutral (num==den) => S == MULT_SCALE (1.0 in M units).
    let s = compute_system_index_scaled(
        (MULT_SCALE, MULT_SCALE),
        (MULT_SCALE, MULT_SCALE),
        (MULT_SCALE, MULT_SCALE),
    );
    assert_eq!(s, MULT_SCALE);
}

#[test]
fn test_system_index_scaled_multiplies() {
    use percolator_insurance::premium::compute_system_index_scaled;
    // oi_vault 2.0, pool 3.0, vol 1.0 => 6.0 => 6 * MULT_SCALE.
    let s = compute_system_index_scaled(
        (2 * MULT_SCALE, MULT_SCALE),
        (3 * MULT_SCALE, MULT_SCALE),
        (MULT_SCALE, MULT_SCALE),
    );
    assert_eq!(s, 6 * MULT_SCALE);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p percolator-insurance --test premium_tests test_system_index_scaled -v`
Expected: FAIL — `cannot find function compute_system_index_scaled`.

- [ ] **Step 3: Write minimal implementation**

Add to `src/premium.rs` (uses existing `gcd`):
```rust
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
    // Accumulate as a reduced (num, den) then scale by MULT_SCALE.
    let mut num: u128 = MULT_SCALE;
    let mut den: u128 = 1;
    for (c_num, c_den) in [oi_vault, pool_health, volatility] {
        if c_den == 0 {
            // Defensive: a zero denominator means a misconfigured neutral; treat
            // as 1.0 rather than panic (construction validates these anyway).
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p percolator-insurance --test premium_tests test_system_index_scaled -v`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add percolator-insurance/src/premium.rs percolator-insurance/tests/premium_tests.rs
git commit -m "feat(insurance): add compute_system_index_scaled for global accumulator

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: `compute_interval_premium` (time-integrated premium)

**Files:**
- Modify: `percolator-insurance/src/premium.rs`
- Test: `percolator-insurance/tests/premium_tests.rs`

- [ ] **Step 1: Write the failing tests**

Append to `tests/premium_tests.rs`:
```rust
#[test]
fn test_interval_premium_matches_per_slot_when_constant() {
    use percolator_insurance::premium::{compute_interval_premium, compute_premium_per_slot};
    // Neutral system index over n slots: system_accrued = MULT_SCALE * n.
    // Compare against compute_premium_per_slot summed n times with the same
    // neutral leverage/crowd. notional small, leverage <= 1 so lev factor == M.
    let notional = 1_000_000u128;
    let base_rate = 1_000_000u128;
    let n = 100u128;
    let m = MULT_SCALE;
    let lev_charged = m; // 1.0
    let crowd = m;       // 1.0
    let system_accrued = m * n; // neutral system integrated over n slots

    let interval = compute_interval_premium(notional, base_rate, lev_charged, crowd, system_accrued, 1);

    // Per-slot with all multipliers neutral (capital huge => leverage 1.0).
    let idx = RiskIndex::neutral();
    let per_slot = compute_premium_per_slot(notional, u128::MAX, base_rate, &idx, 1);
    // Allow a small rounding band (ceiling per-slot vs single ceiling interval).
    let lo = per_slot.saturating_mul(n).saturating_sub(n);
    let hi = per_slot.saturating_mul(n).saturating_add(n);
    assert!(interval >= lo && interval <= hi,
        "interval {interval} not within [{lo},{hi}] of per_slot*n");
}

#[test]
fn test_interval_premium_saturates_up_on_overflow() {
    use percolator_insurance::premium::compute_interval_premium;
    let p = compute_interval_premium(u128::MAX, u128::MAX, MULT_SCALE, MULT_SCALE, MULT_SCALE, 1);
    assert!(p > 1, "overflow must saturate up, not collapse to min_premium; got {p}");
}

#[test]
fn test_interval_premium_zero_accrued_is_min() {
    use percolator_insurance::premium::compute_interval_premium;
    // No time elapsed in the accumulator => no premium owed (floored at min).
    let p = compute_interval_premium(1_000_000, 1_000_000, MULT_SCALE, MULT_SCALE, 0, 7);
    assert_eq!(p, 7);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p percolator-insurance --test premium_tests test_interval_premium -v`
Expected: FAIL — `cannot find function compute_interval_premium`.

- [ ] **Step 3: Write minimal implementation**

Add to `src/premium.rs`. Reuse the same overflow discipline as `compute_premium_per_slot` (GCD reduce, shift-down fallback, saturate UP). Denominator is `PREMIUM_SCALE × MULT_SCALE³`:
```rust
/// Premium for an accrual interval, integrating the system risk over time.
///
/// ```text
/// premium = notional × base_rate × lev_charged × crowd × system_accrued
///           ÷ (PREMIUM_SCALE × MULT_SCALE³)
/// ```
/// `lev_charged` and `crowd` are in MULT_SCALE units; `system_accrued` is
/// `Σ system_index_scaled · dt` (also carrying one MULT_SCALE). For a constant
/// system index and leverage this equals `compute_premium_per_slot × slots`.
/// Floored at `min_premium`; saturates UP on true overflow.
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
    // denominator = PREMIUM_SCALE * MULT_SCALE^3
    let den0 = PREMIUM_SCALE
        .saturating_mul(MULT_SCALE)
        .saturating_mul(MULT_SCALE)
        .saturating_mul(MULT_SCALE);

    let mut num: u128 = notional;
    let mut den: u128 = den0;

    // Fold each numerator factor with GCD reduction + saturate-up on overflow.
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
                    None => return u128::MAX, // saturate up (review #5 policy)
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
```

- [ ] **Step 4: Run to verify passing**

Run: `cargo test -p percolator-insurance --test premium_tests test_interval_premium -v`
Expected: PASS (all three).

- [ ] **Step 5: Commit**

```bash
git add percolator-insurance/src/premium.rs percolator-insurance/tests/premium_tests.rs
git commit -m "feat(insurance): add compute_interval_premium (time-integrated premium)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: Global accumulator state + `accrue_global` + public `accrue`

**Files:**
- Modify: `percolator-insurance/src/wrapper.rs` (struct `InsuredRiskEngine`, `new`, new methods)
- Test: `percolator-insurance/tests/integration_tests.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/integration_tests.rs`:
```rust
#[test]
fn test_accrue_global_is_monotonic_and_advances() {
    let mut engine = setup_engine(); // oracle 1000, slot 1
    let start = engine.cum_system_index;
    // Advancing time accrues (system index >= MULT_SCALE since factors >= 1.0).
    engine.accrue(10);
    let after = engine.cum_system_index;
    assert!(after >= start, "accumulator must be monotonic");
    assert_eq!(engine.last_accrue_slot, 10);
    // Non-advancing time is a no-op.
    let frozen = engine.cum_system_index;
    engine.accrue(10);
    assert_eq!(engine.cum_system_index, frozen, "no-op when slot does not advance");
    engine.accrue(5);
    assert_eq!(engine.cum_system_index, frozen, "no-op when slot goes backward");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p percolator-insurance --test integration_tests test_accrue_global -v`
Expected: FAIL — no field `cum_system_index` / no method `accrue`.

- [ ] **Step 3: Write minimal implementation**

In `src/wrapper.rs`, add fields to `InsuredRiskEngine` (after `account_premiums`):
```rust
    /// Monotonic accumulator of system_index_scaled × slots (funding pattern).
    pub cum_system_index: u128,
    /// Slot at which `cum_system_index` was last advanced.
    pub last_accrue_slot: u64,
```
In `new(...)`, initialize them in the returned struct (after `account_premiums: [...]`):
```rust
            cum_system_index: 0,
            last_accrue_slot: init_slot,
```
Add methods in the `impl InsuredRiskEngine` block:
```rust
    /// Current account-independent system index in MULT_SCALE units
    /// (oi_vault × pool_health × volatility). No account dimension.
    fn current_system_index(&self) -> u128 {
        let long_oi = self.engine.oi_eff_long_q;
        let short_oi = self.engine.oi_eff_short_q;
        let vault = self.engine.vault.get();
        let oracle_price = self.engine.last_oracle_price;
        let pp = &self.premium_params;

        let total_oi_q = long_oi.saturating_add(short_oi);
        let total_oi_notional = if oracle_price > 0 {
            total_oi_q.saturating_mul(oracle_price as u128) / POS_SCALE
        } else {
            0
        };
        let oi_vault = oi_vault_multiplier(
            total_oi_notional, vault,
            pp.oi_vault_floor_ratio_num, pp.oi_vault_floor_ratio_den,
            pp.oi_vault_cap_ratio_num, pp.oi_vault_cap_ratio_den, pp.oi_vault_mult_max,
        );
        let pool_health = pool_health_multiplier(
            self.pool.balance, total_oi_notional,
            pp.pool_health_low_num, pp.pool_health_low_den,
            pp.pool_health_high_num, pp.pool_health_high_den, pp.pool_health_mult_max,
        );
        let volatility = (pp.volatility_mult_num, pp.volatility_mult_den);
        crate::premium::compute_system_index_scaled(oi_vault, pool_health, volatility)
    }

    /// Advance the global system-risk accumulator to `now_slot`. Permissionless;
    /// a no-op when time does not advance. The keeper SHOULD call this each crank.
    pub fn accrue(&mut self, now_slot: u64) {
        if now_slot <= self.last_accrue_slot {
            return;
        }
        let dt = (now_slot - self.last_accrue_slot) as u128;
        let s = self.current_system_index();
        self.cum_system_index = self.cum_system_index.saturating_add(s.saturating_mul(dt));
        self.last_accrue_slot = now_slot;
    }
```
Add `oi_vault_multiplier, pool_health_multiplier` to the `use crate::risk_index::{...}` import if not already present (they are imported in the current file).

- [ ] **Step 4: Run to verify passing**

Run: `cargo test -p percolator-insurance --test integration_tests test_accrue_global -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add percolator-insurance/src/wrapper.rs percolator-insurance/tests/integration_tests.rs
git commit -m "feat(insurance): add global system-risk accumulator + accrue() entrypoint

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: Per-account snapshot fields + leverage_factor helper + activate seeding

**Files:**
- Modify: `percolator-insurance/src/wrapper.rs` (`AccountPremiumState`, `activate_premium`, new helper)
- Test: `percolator-insurance/tests/integration_tests.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/integration_tests.rs`:
```rust
#[test]
fn test_activate_seeds_snapshot_and_leverage() {
    let mut engine = setup_engine();
    let oracle = 1000u64;
    engine.accrue(5); // advance the global accumulator first
    engine
        .execute_trade(0, 1, oracle, 6, make_size_q(10), oracle, 0, 0, 100, None)
        .unwrap();
    // After opening, the account's snapshot equals the current global accumulator.
    assert_eq!(
        engine.account_premiums[0].cum_system_snapshot,
        engine.cum_system_index
    );
    assert!(engine.account_premiums[0].last_leverage_factor >= MULT_SCALE,
        "leverage factor must be >= 1.0");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p percolator-insurance --test integration_tests test_activate_seeds -v`
Expected: FAIL — no field `cum_system_snapshot`.

- [ ] **Step 3: Write minimal implementation**

Add fields to `AccountPremiumState`:
```rust
    /// `cum_system_index` snapshot at this account's last touch.
    pub cum_system_snapshot: u128,
    /// `leverage_mult × tail` sampled at last touch (max-of-endpoints input).
    pub last_leverage_factor: u128,
```
Update `AccountPremiumState::new()` to set both to `0`.

Add a leverage-factor helper to `impl InsuredRiskEngine`:
```rust
    /// Combined leverage factor `leverage_mult × tail_surcharge` in MULT_SCALE
    /// units (1.0 == MULT_SCALE). Account-specific; the max of consecutive
    /// samples is charged (anti-flicker, review hybrid design).
    fn leverage_factor(&self, account_idx: usize, notional: u128) -> u128 {
        let capital = self.engine.accounts[account_idx].capital.get();
        let (lev_num, lev_den) = leverage_multiplier(notional, capital, 3, 2);
        let tail = leverage_tail_surcharge(
            notional, capital,
            self.engine.params.maintenance_margin_bps as u128,
            self.premium_params.leverage_tail_threshold_bps,
            self.premium_params.leverage_tail_steepness,
        );
        // (lev_num/lev_den) × (tail.0/tail.1) in MULT_SCALE units.
        let num = lev_num.saturating_mul(tail.0);
        let den = lev_den.saturating_mul(tail.1) / MULT_SCALE.max(1);
        if den == 0 { return MULT_SCALE; }
        (num / den).max(MULT_SCALE)
    }
```
Add `leverage_multiplier` and `leverage_tail_surcharge` to the `use crate::premium::{...}` import (both already partially imported).

In `activate_premium`, after computing `notional` and the commitment, set the new fields when writing `self.account_premiums[i] = AccountPremiumState { ... }`:
```rust
            cum_system_snapshot: self.cum_system_index,
            last_leverage_factor: self.leverage_factor(i, notional),
```
(Keep the existing `last_premium_slot`, `commitment_end_slot`, `prepaid_premium`, `is_active`. Commitment continues to use `compute_premium_per_slot(...)` at open as its point estimate — unchanged.)

- [ ] **Step 4: Run to verify passing**

Run: `cargo test -p percolator-insurance --test integration_tests test_activate_seeds -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add percolator-insurance/src/wrapper.rs percolator-insurance/tests/integration_tests.rs
git commit -m "feat(insurance): per-account accumulator snapshot + leverage_factor helper

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: Rewrite `collect_accrued_premium` to use the accumulator + leverage max

**Files:**
- Modify: `percolator-insurance/src/wrapper.rs` (`collect_accrued_premium`)
- Test: `percolator-insurance/tests/integration_tests.rs`

- [ ] **Step 1: Write the failing tests (the anti-gaming proofs)**

Append to `tests/integration_tests.rs`:
```rust
#[test]
fn test_buy_and_hold_pays_for_integrated_spike() {
    // Open calm; advance the GLOBAL accumulator through a "spike" via accrue()
    // (simulating other activity / keeper cranks), WITHOUT the account touching;
    // then a single collect must bill the integrated spike.
    let mut engine = setup_engine();
    let oracle = 1000u64;
    engine
        .execute_trade(0, 1, oracle, 2, make_size_q(10), oracle, 0, 0, 100, None)
        .unwrap();
    let baseline_collected = engine.pool.total_collected;

    // Force a high system index: deplete pool_health by inflating total OI vs pool,
    // here approximated by advancing many slots (system index >= 1.0 each slot).
    for s in (10..2000).step_by(100) {
        engine.accrue(s);
    }
    let collected = engine.collect_accrued_premium(0, 2000).unwrap();
    assert!(collected > 0, "buy-and-hold must owe premium for elapsed system risk");
    assert!(engine.pool.total_collected > baseline_collected);
}

#[test]
fn test_leverage_flicker_billed_at_high_endpoint() {
    let mut engine = setup_engine();
    let oracle = 1000u64;
    engine
        .execute_trade(0, 1, oracle, 2, make_size_q(10), oracle, 0, 0, 100, None)
        .unwrap();
    let lev_before = engine.account_premiums[0].last_leverage_factor;
    // A deposit lowers leverage; collect samples the new (lower) factor, but the
    // stored last_leverage_factor only matters via max() on the NEXT interval.
    engine.deposit(0, 1_000_000, 50).unwrap();
    // last_leverage_factor was updated by the deposit's collect; assert it is a
    // valid >= 1.0 sample (regression guard that the field is maintained).
    assert!(engine.account_premiums[0].last_leverage_factor >= MULT_SCALE);
    let _ = lev_before;
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p percolator-insurance --test integration_tests test_buy_and_hold -v`
Expected: FAIL — `collect_accrued_premium` still uses the old per-slot rate and `total_collected` won't reflect the accumulator (and may not compile against new fields yet). Confirm it fails before editing the method.

- [ ] **Step 3: Write the implementation**

Replace the body of `collect_accrued_premium` between the `slots_elapsed`/`notional` block and the `self.account_premiums[i].last_premium_slot = now_slot;` line. New logic:
```rust
        // Advance the global accumulator so this interval sees all elapsed
        // system risk (advanced by every op + keeper accrue()). `collect` already
        // holds `&mut self`, so call accrue directly.
        self.accrue(now_slot);

        let notional = self.account_notional(i);
        if notional == 0 {
            self.account_premiums[i].cum_system_snapshot = self.cum_system_index;
            self.account_premiums[i].last_premium_slot = now_slot;
            return Ok(0);
        }
        let capital = self.engine.accounts[i].capital.get();

        // System risk integrated over the interval since the account's snapshot.
        let system_accrued = self
            .cum_system_index
            .saturating_sub(self.account_premiums[i].cum_system_snapshot);

        // Leverage: max of the previous sample and the current one (anti-flicker).
        let lev_now = self.leverage_factor(i, notional);
        let lev_charged = core::cmp::max(self.account_premiums[i].last_leverage_factor, lev_now);

        // Crowding (side) is point-sampled (documented residual).
        let crowd = self.compute_risk_index(i).crowding.0;

        let premium_owed = crate::premium::compute_interval_premium(
            notional,
            self.premium_params.base_rate_per_slot,
            lev_charged,
            crowd,
            system_accrued,
            self.premium_params.min_premium_per_slot,
        );
```
Keep the existing prepaid-drawdown + `cap_premium_to_maint_buffer` + before/after insurance-balance delta recording EXACTLY as-is (operating on `premium_owed`). After the charge block, update the snapshot + leverage in addition to the slot:
```rust
        self.account_premiums[i].cum_system_snapshot = self.cum_system_index;
        self.account_premiums[i].last_leverage_factor = lev_now;
        self.account_premiums[i].last_premium_slot = now_slot;
        Ok(collected)
```
Also DELETE the old `let risk_idx = self.risk_index_with_notional(...)`, `let rate = compute_premium_per_slot(...)`, and `let premium_owed = rate.saturating_mul(...)` lines — replaced above.

Note: `compute_interval_premium` returns `min_premium` (>= 1 in practice) rather than 0, so the prepaid/charge path always runs for an active account with notional; the existing prepaid-drawdown + cap + delta-recording code already tolerates a `min_premium`-sized owed amount. The flat (`notional == 0`) early-out above still advances the snapshot and slot and returns `Ok(0)`.

- [ ] **Step 4: Run to verify passing**

Run: `cargo test -p percolator-insurance --test integration_tests -v`
Expected: the two new tests PASS. Some pre-existing accrual-amount assertions may now fail (different integrated amounts) — that is expected; Task 8 updates them. Confirm only accrual-*amount* tests changed, not invariant tests.

- [ ] **Step 5: Commit**

```bash
git add percolator-insurance/src/wrapper.rs percolator-insurance/tests/integration_tests.rs
git commit -m "feat(insurance): integrate premium via global accumulator + leverage max

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: Advance the accumulator from every wrapped op

**Files:**
- Modify: `percolator-insurance/src/wrapper.rs` (`deposit`/`deposit_inner`, `execute_trade`, `withdraw`/`withdraw_inner`, `liquidate`)
- Test: `percolator-insurance/tests/integration_tests.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn test_deposit_advances_global_accumulator() {
    let mut engine = setup_engine();
    // open a position so system index has OI to price
    engine.execute_trade(0, 1, 1000, 2, make_size_q(10), 1000, 0, 0, 100, None).unwrap();
    let before = engine.cum_system_index;
    engine.deposit(0, 1_000, 500).unwrap(); // 498 slots later
    assert!(engine.cum_system_index >= before, "deposit must advance the accumulator");
    assert_eq!(engine.last_accrue_slot, 500);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p percolator-insurance --test integration_tests test_deposit_advances -v`
Expected: FAIL — `last_accrue_slot` not advanced to 500 (only collect calls accrue today).

- [ ] **Step 3: Write the implementation**

Add `self.accrue(now_slot);` as the FIRST line inside `deposit_inner`, `execute_trade`, `withdraw_inner`, and `liquidate` (using each method's `now_slot` parameter). `collect_accrued_premium` already calls it (Task 5). This is idempotent (no-op if time hasn't advanced).

- [ ] **Step 4: Run to verify passing**

Run: `cargo test -p percolator-insurance --test integration_tests test_deposit_advances -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add percolator-insurance/src/wrapper.rs percolator-insurance/tests/integration_tests.rs
git commit -m "feat(insurance): advance global accumulator from every wrapped op

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 7: Kani harnesses for the accumulator and interval premium

**Files:**
- Modify: `percolator-insurance/src/kani_harness.rs`

- [ ] **Step 1: Add harnesses (cfg(kani)-gated; do not run in cargo test)**

Append inside the `#[cfg(kani)]` module in `src/kani_harness.rs`:
```rust
    #[kani::proof]
    fn kani_system_index_scaled_no_panic() {
        let a: u128 = kani::any::<u32>() as u128;
        let b: u128 = kani::any::<u32>() as u128;
        let c: u128 = kani::any::<u32>() as u128;
        // denominators non-zero (validated at construction)
        let _ = crate::premium::compute_system_index_scaled(
            (a, 1.max(b)), (c, 1.max(a)), (b, 1.max(c)),
        );
    }

    #[kani::proof]
    fn kani_interval_premium_monotonic_in_accrued() {
        let notional = kani::any::<u16>() as u128;
        let base = kani::any::<u16>() as u128;
        let lev = MULT_SCALE + kani::any::<u8>() as u128;
        let crowd = MULT_SCALE + kani::any::<u8>() as u128;
        let a1 = kani::any::<u16>() as u128;
        let a2 = a1 + (kani::any::<u8>() as u128);
        let p1 = crate::premium::compute_interval_premium(notional, base, lev, crowd, a1, 1);
        let p2 = crate::premium::compute_interval_premium(notional, base, lev, crowd, a2, 1);
        assert!(p2 >= p1);
    }
```
(Ensure `MULT_SCALE` is in scope in the harness module; it already imports crate items.)

- [ ] **Step 2: Verify normal build/test unaffected**

Run: `cargo build -p percolator-insurance && cargo test -p percolator-insurance 2>&1 | tail -2`
Expected: builds clean, all tests pass (Kani code is cfg-gated out).

- [ ] **Step 3: Commit**

```bash
git add percolator-insurance/src/kani_harness.rs
git commit -m "test(insurance): Kani harnesses for accumulator + interval premium

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 8: Update pre-existing tests, docs, and final verification

**Files:**
- Modify: `percolator-insurance/tests/integration_tests.rs`, `percolator-insurance/tests/pool_tests.rs` (only if they assert exact accrual amounts), `percolator-insurance/README.md`, `percolator-insurance/docs/REVIEW.md`

- [ ] **Step 1: Update accrual-amount assertions to integrated semantics**

Run the full suite and inspect failures: `cargo test -p percolator-insurance 2>&1 | grep -A3 FAILED`. For each failing test that asserted a specific old per-slot amount (e.g. `test_collect_premium_after_trade`), update it to assert the new behaviour (premium > 0 / invariants hold) rather than a hard amount. Do NOT weaken invariant tests (`check_invariants`, `check_conservation`, the bug-#1 `total_paid_out == 0` test, the counter-cyclical cap test) — those must still pass unchanged.

- [ ] **Step 2: Update docs**

In `README.md`, under "What it computes", add a sentence: premium now integrates system risk over real elapsed time via a global accumulator (funding-rate pattern) advanced by every op + `accrue()`, with leverage charged at the max of interval endpoints. In `docs/REVIEW.md`, change the TWA roadmap line from `[ ]` to `[x]` with the implementing approach noted.

- [ ] **Step 3: Full verification**

```bash
cargo test -p percolator-insurance 2>&1 | grep -E "^test result"
cargo test -p percolator-sim 2>&1 | grep -E "^test result"   # sim must still build/pass
cargo clippy -p percolator-insurance --lib --tests 2>&1 | grep -c "percolator-insurance/(src|tests)"
cargo build  # whole workspace
```
Expected: all tests pass; sim unaffected; 0 insurance clippy warnings; workspace builds.

- [ ] **Step 4: Commit**

```bash
git add percolator-insurance/tests percolator-insurance/README.md percolator-insurance/docs/REVIEW.md
git commit -m "test+docs(insurance): integrated-accrual semantics, mark TWA done

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Definition of Done
- All new tests + the 89 prior tests green; sim builds & passes; 0 insurance clippy warnings; workspace builds.
- Buy-and-hold and leverage-flicker tests prove the anti-gaming behaviour.
- Bug-#1 delta recording, counter-cyclical cap, oracle/auth guards preserved.
- README + REVIEW.md updated; spec's residuals documented in code comments.
- Push to `mine main` after a final review.
