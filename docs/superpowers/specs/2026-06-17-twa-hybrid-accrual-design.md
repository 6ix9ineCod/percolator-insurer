# Hybrid TWA Premium Accrual — Design Spec

**Date:** 2026-06-17
**Crate:** `percolator-insurance`
**Status:** Approved design, pre-implementation

## 1. Problem

The actuarial review identified that premium accrual is gameable because **risk is
priced at discrete sample points the attacker influences**. Premium accrues for
every slot between an account's touches, but the *rate* is computed instantaneously
at the collection touch. Three gaming windows follow:

1. **Rate-timing** — ensure the collecting touch lands at a low-index instant; the
   whole elapsed interval is billed at that low rate.
2. **Commitment lock-in** — the prepaid commitment is priced at open-time index;
   open cheap, carry through a spike, never re-priced.
3. **Leverage flicker** — deposit to suppress instantaneous leverage around the
   charge, then withdraw.

The canonical attack — **open when calm, hold through a risk spike, close** — makes
exactly one touch (the close) and defeats any "sample at this account's touches"
scheme, including left-/right-/max-sampling of a per-account rate.

## 2. Root cause and chosen approach

Root cause: sampling at adversary-chosen points. The fix must price the fast,
shared, gameable risk dimension over **real elapsed time**, not over the attacker's
touches.

**Chosen design (hybrid):**
- **A global cumulative accumulator** (the funding-rate pattern) integrates the
  account-*independent* system risk over time. It is advanced by *every* account's
  touch and by the existing keeper crank, so no single account can starve it.
- **Max-of-endpoints** on the per-account leverage factor hardens the leverage
  flicker game.

Rejected alternatives: per-account left-sampled accumulator (relocates the game,
fails the buy-and-hold attack); per-slot per-account keeper poke (gas-prohibitive,
new infra). The global accumulator reuses the crank cadence a perp already runs.

## 3. Factor split

The per-slot premium is
`notional × base_rate × lev × tail × crowd × oi_vault × pool_health × volatility / scales`.

| Factor | Class | Handling |
|---|---|---|
| `oi_vault`, `pool_health`, `volatility` | account-independent (shared) | **integrated** in the global accumulator |
| `base_rate` | constant | scalar, applied at collection |
| `lev × tail` | account-specific, gameable (flicker) | **max-of-endpoints** |
| `crowd` (side) | account-specific, slow-moving | point-sampled at collection (documented residual) |
| `notional` | account-specific, oracle-driven | point-sampled at collection |

## 4. State

**Engine-global (`InsuredRiskEngine`):**
- `cum_system_index: u128` — monotonic accumulator of `system_index × slots`.
- `last_accrue_slot: u64` — slot at which the accumulator was last advanced.

**Per-account (`AccountPremiumState`):**
- `cum_system_snapshot: u128` — value of `cum_system_index` at this account's last touch.
- `last_leverage_factor: u128` — `lev × tail` sampled at this account's last touch (for max-of-endpoints).

`accrual_rate`/`max_accrual_rate` from the rejected design are NOT added.

## 5. Arithmetic (pure integer, no_std, overflow-safe)

Let `M = MULT_SCALE`, `P = PREMIUM_SCALE`. Multipliers are `(num, den)` with `den = M` at 1.0.

**System index per tick** (`compute_system_index_scaled`), account-independent, in `M` units:
```
S = oi_vault_mult × pool_health_mult × volatility_mult × M
  = (oiv_num/oiv_den) × (pool_num/pool_den) × (vol_num/vol_den) × M    // GCD-reduced, saturating
```
`S >= M` when all factors >= 1.0. Computed once per tick (no account index).

**Accumulator tick** (`accrue_global`):
```
dt = now − last_accrue_slot
cum_system_index = cum_system_index.saturating_add(S.saturating_mul(dt))
last_accrue_slot = now
```
Growth ≈ `S·dt`; with `S ~ 10^4·M` and `dt ~ 3·10^8 slots/yr`, `cum` grows ~`10^15/yr`,
so a `u128` lasts ~`10^23` years. Saturating add is the hard ceiling.

**Interval premium** (`compute_interval_premium`):
```
system_accrued = cum_system_index_now − cum_system_snapshot           // (M-units × slots)
lev_charged    = max(last_leverage_factor, lev_now)                    // M-units (lev × tail)
premium = notional × base_rate × lev_charged × crowd_now × system_accrued
          ÷ (P × M^3)
```
Derivation: `lev_charged/M`, `crowd/M`, and `system_accrued/M = Σ(system_mult·dt)`,
so `premium = notional × (base_rate/P) × (lev/M) × (crowd/M) × Σ(system_mult·dt)`.
For a *constant* system index over `n` slots this equals `per_slot_premium × n`
(behaviour-preservation invariant). Implemented with the same GCD-reduction +
saturate-up-on-overflow machinery as `compute_premium_per_slot` (review #5).

## 6. Control flow

`accrue_global(now)` is called at the **top of every** wrapped op
(`deposit`, `execute_trade`, `withdraw`, `liquidate`, `collect_accrued_premium`)
and from a new **public `accrue(now)`** the keeper invokes alongside its crank.

`collect_accrued_premium(idx, now)`:
1. Guard `is_active`, `now > last_premium_slot`; `accrue_global(now)`.
2. `notional = account_notional(i)`; flat early-out (review #4).
3. `system_accrued = cum_system_index − cum_system_snapshot`.
4. `lev_now = leverage_factor(notional, capital)`; `lev_charged = max(last_leverage_factor, lev_now)`.
5. `premium_owed = compute_interval_premium(notional, base_rate, lev_charged, crowd_now, system_accrued, min_premium)`.
6. Draw `prepaid`, then charge capital — **counter-cyclical cap + bug-#1 actual-delta recording unchanged**.
7. `cum_system_snapshot = cum_system_index`; `last_leverage_factor = lev_now`; `last_premium_slot = now`.

`activate_premium` seeds `cum_system_snapshot = cum_system_index`,
`last_leverage_factor = lev_open`, and prices the commitment as
`min_commitment_slots × compute_premium_per_slot(...at open...)` — the existing
per-slot function is retained solely as the commitment's point estimate. The
commitment is now a floor only; the accumulator is the primary anti-gaming
mechanism.

## 7. Error handling

- All arithmetic `checked_`/`saturating_`; overflow saturates **up** (conservative),
  consistent with review #5.
- `accrue_global` is a no-op when `now <= last_accrue_slot` (monotonic time).
- Param validation extends `new()` (zero denominators already rejected).
- No new panics; preserve `#![forbid(unsafe_code)]`, no float.

## 8. Residual limitations (documented in code)

- **Crowding side** is point-sampled (slow, engine-bounded; not the fast shared dimension).
- **Sampling density = touch + keeper-crank cadence.** Robust when a keeper cranks
  regularly; coarse if activity is sparse *and* no keeper runs. The integrator
  SHOULD call `accrue()` on each crank. This is the inherent trade vs the rejected
  per-slot poke.
- **Notional** uses the current oracle price (oracle-driven, not trader-controlled).

## 9. Testing (TDD)

1. **Buy-and-hold attack**: open at low system index; advance `cum_system_index`
   via *other* activity through a spike; the attacker's single close-touch must bill
   the *integrated* spike, not the calm open rate.
2. **Leverage flicker**: deposit-lower then withdraw-raise; the interval bills at the
   high endpoint (`max`).
3. **Behaviour preservation**: constant system index *and* constant leverage ⇒
   interval premium == `compute_premium_per_slot × slots` within ceiling rounding.
4. **Accumulator**: monotonic, no-op on non-advancing time, saturating overflow.
5. **Regression**: bug-#1 delta, counter-cyclical cap, oracle/auth guards, and the
   89 existing tests stay green (accrual-amount tests updated to integrated semantics).
6. **Kani**: `cum_system_index` monotonic; `compute_interval_premium` never panics
   and is monotonic non-decreasing in `notional` and `system_accrued`.

## 10. Out of scope

Crowding-side TWA, leverage TWA beyond max-of-endpoints, base_rate/vol calibration
(separate tracked items), and any change to the parent engine.
