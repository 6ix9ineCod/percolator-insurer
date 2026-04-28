# Insurance Premium Pool — Design Specification

**Date:** 2026-04-27
**Status:** Approved
**Scope:** v1 — on-chain premium engine with separate pool, wrapper crate architecture

## 1. Problem Statement

Percolator's risk engine guarantees solvency through a haircut ratio that scales
profit payouts to match actual vault balance. The insurance fund absorbs deficits
from bankrupt accounts, but it is finite — a correlated blowup (mass liquidation
cascade) can drain it entirely, forcing the haircut to activate across all
profitable accounts.

The haircut is a mathematically sound backstop, but a poor primary defense:
- All winners share the penalty equally regardless of who caused the risk
- There is no cost signal for reckless leverage — a 100x trader imposes the same
  systemic cost as a 5x trader until the moment of liquidation
- The insurance fund grows only from liquidation fees and wrapper top-ups, not
  from the ongoing risk that open positions impose

This extension adds a **continuous, risk-priced insurance premium** that:
1. Charges each position proportionally to the risk it creates
2. Pools premiums separately from Percolator's insurance fund
3. Feeds Percolator's fund when deficit is detected — before the haircut activates
4. Makes the haircut the emergency backstop rather than the primary mechanism

## 2. Architecture

### 2.1 Deployment Model

**Wrapper crate** (`percolator-insurance`) that imports Percolator as a
dependency. All interaction with Percolator goes through its public API. No
access to private fields or methods.

**Hybrid on-chain/off-chain model:**
- v1: All computation on-chain. Risk signals derived from Percolator's public
  state fields.
- v2: Off-chain co-processor feeds a volatility oracle for the fourth risk
  signal.

### 2.2 Module Structure

```
percolator-insurance/
  Cargo.toml
  src/
    lib.rs            — public API facade, re-exports
    premium.rs        — premium calculation (pure math, no side effects)
    pool.rs           — PremiumPool state management and deficit detection
    risk_index.rs     — systemic risk index from on-chain signals
    wrapper.rs        — InsuredRiskEngine orchestrator
```

### 2.3 Dependency Boundary

The wrapper reads the following public fields from `RiskEngine`:

| Field | Used For |
|-------|----------|
| `engine.oi_eff_long_q` | Crowding multiplier |
| `engine.oi_eff_short_q` | Crowding multiplier |
| `engine.vault.get()` | OI/vault multiplier, deficit detection |
| `engine.c_tot.get()` | Deficit detection |
| `engine.insurance_fund.balance.get()` | Deficit detection |
| `engine.pnl_matured_pos_tot` | Deficit threshold |
| `engine.last_oracle_price` | Notional calculation |
| `engine.current_slot` | Premium accrual timing |
| `engine.accounts[idx].capital.get()` | Leverage calculation |
| `engine.accounts[idx].position_basis_q` | Side detection, notional |

The wrapper calls the following public methods:

| Method | Used For |
|--------|----------|
| `charge_account_fee_not_atomic` | Collecting premiums from accounts (routes to insurance fund) |
| `top_up_insurance_fund` | Routing external funds into Percolator's fund (v2 tiered reserves) |
| `execute_trade_not_atomic` | Trade execution (pass-through) |
| `deposit_not_atomic` | Deposit (pass-through) |
| `withdraw_not_atomic` | Withdrawal with premium gate |
| `liquidate_at_oracle_not_atomic` | Liquidation with deficit check |
| `settle_account_not_atomic` | Settlement with premium collection |
| `haircut_ratio` | Read vault health for diagnostics and pool health signal |
| `check_conservation` | Post-operation invariant verification |

## 3. Data Structures

### 3.1 PremiumParams

Deploy-time configuration. Immutable after initialization.

```rust
pub struct PremiumParams {
    /// Base premium rate per slot per unit notional.
    /// Denominated in token units scaled by PREMIUM_SCALE.
    /// Example: 100 = 100 / 1e9 per slot per unit notional.
    pub base_rate_per_slot: u128,

    /// Leverage exponent as a rational number.
    /// leverage_multiplier = leverage ^ (num / den).
    /// Example: num=3, den=2 gives exponent 1.5.
    pub leverage_exponent_num: u64,
    pub leverage_exponent_den: u64,

    /// Minimum premium commitment in slots.
    /// Charged upfront when a position is opened.
    /// 216,000 slots = ~24 hours at 400ms per slot.
    pub min_commitment_slots: u64,

    /// Crowding multiplier floor (scaled by systemic_risk_scale).
    /// Applied when long/short ratio <= 1.5.
    pub crowding_floor: u128,

    /// Crowding multiplier cap (scaled by systemic_risk_scale).
    /// Applied when long/short ratio >= 5.0 or one side is empty.
    pub crowding_cap: u128,

    /// Crowding ratio thresholds (as num/den pairs).
    /// Below low_ratio: floor applies. Above high_ratio: cap applies.
    pub crowding_ratio_low_num: u128,
    pub crowding_ratio_low_den: u128,
    pub crowding_ratio_high_num: u128,
    pub crowding_ratio_high_den: u128,

    /// OI/vault multiplier bounds (scaled by systemic_risk_scale).
    /// system_leverage = total_oi_notional / vault.
    pub oi_vault_floor_ratio_num: u128,
    pub oi_vault_floor_ratio_den: u128,
    pub oi_vault_cap_ratio_num: u128,
    pub oi_vault_cap_ratio_den: u128,
    pub oi_vault_mult_max: u128,

    /// Pool health multiplier bounds (scaled by systemic_risk_scale).
    /// health = pool_balance / total_oi_notional.
    pub pool_health_low_num: u128,
    pub pool_health_low_den: u128,
    pub pool_health_high_num: u128,
    pub pool_health_high_den: u128,
    pub pool_health_mult_max: u128,

    /// Common denominator for all multiplier scaling.
    pub systemic_risk_scale: u128,

    /// Minimum premium per slot (floor).
    /// Prevents dust positions from paying zero premium.
    pub min_premium_per_slot: u128,

    /// Deficit detection threshold.
    /// Pool tops up when: residual < pnl_matured_pos_tot * threshold_num / threshold_den.
    pub deficit_threshold_num: u128,
    pub deficit_threshold_den: u128,
}
```

### 3.2 PremiumPool

Pool state. Mutated by the wrapper on every premium collection and deficit
top-up.

```rust
pub struct PremiumPool {
    /// Current pool balance in token units.
    pub balance: u128,

    /// Lifetime premiums collected (monotonically increasing).
    pub total_collected: u128,

    /// Lifetime payouts to Percolator's insurance fund (monotonically increasing).
    pub total_paid_out: u128,

    /// Slot of last deficit check.
    pub last_deficit_check_slot: u64,
}
```

**Invariants (checked after every mutation):**
1. `balance <= total_collected - total_paid_out`
2. `total_paid_out <= total_collected`

### 3.3 AccountPremiumState

Per-account premium tracking. Parallel array indexed identically to
Percolator's `accounts[]`.

```rust
pub struct AccountPremiumState {
    /// Slot when premium was last collected for this account.
    pub last_premium_slot: u64,

    /// Slot when the minimum commitment period expires.
    /// Set to `open_slot + min_commitment_slots` on position open.
    pub commitment_end_slot: u64,

    /// Upfront premium paid at position open (the 24h commitment amount).
    /// Decremented as slots elapse. If the position closes before commitment
    /// expires, the remainder stays in the pool.
    pub prepaid_premium: u128,

    /// Whether this account has an active position paying premiums.
    pub is_active: bool,
}
```

### 3.4 InsuredRiskEngine

Top-level wrapper struct.

```rust
pub struct InsuredRiskEngine {
    /// The wrapped Percolator risk engine.
    pub engine: RiskEngine,

    /// Premium pool state.
    pub pool: PremiumPool,

    /// Deploy-time premium parameters.
    pub premium_params: PremiumParams,

    /// Per-account premium state (parallel to engine.accounts).
    pub account_premiums: [AccountPremiumState; MAX_ACCOUNTS],
}
```

## 4. Premium Calculation

### 4.1 Formula

```
premium_per_slot = max(
    (notional × base_rate × lev_num × crowd_num × oiv_num × pool_num)
    ────────────────────────────────────────────────────────────────────,
    (PREMIUM_SCALE × lev_den × crowd_den × oiv_den × pool_den)

    min_premium_per_slot
)
```

Where:
- `PREMIUM_SCALE = 1_000_000_000` (1e9, matches Percolator's FUNDING_DEN)
- All multipliers are `(num, den)` pairs computed by `risk_index.rs`
- Numerator product computed in `u256` to prevent overflow
- Single final division via `wide_mul_div_ceil_u128` (round up — conservative)

### 4.2 Leverage Multiplier

```
leverage = notional / capital
multiplier = leverage ^ (exponent_num / exponent_den)
```

For exponent 1.5 (num=3, den=2):
```
multiplier = leverage ^ (3/2) = leverage × sqrt(leverage)
```

Integer square root via Newton's method (no floating point):
```rust
fn isqrt(n: u128) -> u128 {
    if n == 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
```

For general rational exponents, decompose into integer power and root:
```
leverage^(p/q) = (leverage^p)^(1/q)
```

Using integer nth-root via Newton's method generalized to arbitrary q.

**Scaling:** Leverage is computed as `(notional × LEVERAGE_SCALE) / capital`
where `LEVERAGE_SCALE = 1_000_000`. The multiplier output is scaled by the same
factor.

### 4.3 Crowding Multiplier

Penalizes accounts on the dominant OI side.

```
majority = max(oi_long, oi_short)
minority = min(oi_long, oi_short)
ratio = majority / minority

Account on minority side → multiplier = 1.0 (floor)
Account on majority side:
  ratio <= low_threshold  → multiplier = floor
  ratio >= high_threshold → multiplier = cap
  otherwise               → linear interpolation
```

Interpolation in integer math:
```
multiplier = floor + (ratio - low) × (cap - floor) / (high - low)
```

All terms computed as `(num, den)` — no intermediate division.

When `minority == 0` (fully one-sided market):
- Majority side: `multiplier = cap`
- Flat accounts: `multiplier = floor` (no position, no penalty)

### 4.4 OI/Vault Multiplier

Measures system-level leverage.

```
total_oi_notional = (oi_long + oi_short) × oracle_price / POS_SCALE
system_leverage = total_oi_notional / vault

system_leverage <= floor_ratio → multiplier = 1.0
system_leverage >= cap_ratio   → multiplier = oi_vault_mult_max
otherwise                      → linear interpolation
```

### 4.5 Pool Health Multiplier

Spikes when the premium pool is depleted relative to exposure.

```
health = pool_balance / total_oi_notional

health >= high_threshold → multiplier = 1.0 (pool healthy)
health <= low_threshold  → multiplier = pool_health_mult_max
otherwise                → linear interpolation (inverted — lower health = higher mult)
```

### 4.6 Numerical Validation

Reference values for `base_rate=100, exponent=1.5, balanced market, pool at 4.17% health`:

| Leverage | Lev Mult | Daily Premium ($60k notional) |
|----------|----------|-------------------------------|
| 1.5x | 1.837 | ~$4.80 |
| 5x | 11.18 | ~$29.20 |
| 10x | 31.62 | ~$82.70 |
| 25x | 125.0 | ~$327 |
| 50x | 353.55 | ~$924 |
| 100x | 1,000.0 | ~$2,615 |

Worst case (100x, 4:1 crowded side, maxed OI/vault, depleted pool):
- Combined multiplier: ~60,000x base
- Daily premium: ~$77,838 on $60k notional
- Position drains via premium within hours (by design)

### 4.7 Overflow Safety

Worst-case numerator product:
```
MAX_ACCOUNT_NOTIONAL (1e20) × base_rate (1e9) × lev (1e6) × crowd (4e3) × oiv (3e3) × pool (5e3)
= ~6e45
```

`u256` maximum: ~1.16e77. Headroom factor: ~1.9e31. No overflow possible.

## 5. Premium Pool

### 5.1 Collection

Premiums are collected via Percolator's `charge_account_fee_not_atomic`. This
deducts from account capital and routes to Percolator's insurance fund. The
premium stays inside Percolator's insurance fund — it is not withdrawn.

The `PremiumPool.balance` tracks a **claim** on the insurance fund, not
separate funds. This is an accounting ledger that records how much of the
insurance fund balance originated from premiums, so the wrapper knows how much
it can deploy on deficit top-up vs what came from other sources (liquidation
fees, manual top-ups).

```
Account capital ──charge_account_fee──▶ Percolator InsuranceFund
                                              │
                                              │ (funds stay here)
                                              │
PremiumPool.balance += amount                 │ (accounting only)
PremiumPool.total_collected += amount         │
```

This ensures Percolator's conservation invariant (`V >= C_tot + I`) is
maintained at every step. No vault drawdown from premium collection.

When the insurance fund is drained by `use_insurance_buffer` during ADL,
`PremiumPool.balance` is decremented proportionally to reflect that those
funds were consumed. If the pool has remaining balance and the insurance fund
is depleted, the pool signals that external funds (v2 tiered reserves) should
be routed via `top_up_insurance_fund`.

### 5.2 Deficit Detection and Top-Up

Since premiums stay inside Percolator's insurance fund, deficit detection
monitors whether the fund was drained below the premium pool's tracked claim.

After any operation that can cause a deficit (liquidation, ADL via crank):

```rust
let insurance = engine.insurance_fund.balance.get();

// The pool's claim on the insurance fund exceeds the actual balance.
// This means use_insurance_buffer consumed premium-funded reserves.
if pool.balance > insurance {
    let consumed = pool.balance - insurance;
    pool.balance -= consumed;
    pool.total_paid_out += consumed;
}

// If the insurance fund is fully drained AND the pool still had balance
// before this operation, external reserves (v2) would be needed.
// For v1, the haircut ratio activates as Percolator's backstop.
// The pool's ongoing premium collection rebuilds the fund over time.
```

In v2 (tiered reserves), when the pool detects the insurance fund is drained
and external reserves are available:

```rust
// v2 only — route external reserve funds into Percolator
if insurance == 0 && external_reserves > 0 {
    let top_up = core::cmp::min(deficit, external_reserves);
    engine.top_up_insurance_fund(top_up, now_slot)?;
    // external_reserves -= top_up handled by tiered reserve manager
}
```

### 5.3 Pool Invariants

After every mutation:
1. `balance + total_paid_out == total_collected` (conservation)
2. `total_paid_out <= total_collected` (monotonicity)
3. `balance <= engine.insurance_fund.balance.get()` (claim never exceeds actual fund)
4. No operation creates a negative balance

## 6. Systemic Risk Index

### 6.1 Signals (v1 — On-Chain Only)

| Signal | Source | Multiplier Range |
|--------|--------|------------------|
| Long/short imbalance | `oi_eff_long_q`, `oi_eff_short_q` | 1.0 – `crowding_cap` |
| System leverage | `(oi_long + oi_short) × price / vault` | 1.0 – `oi_vault_mult_max` |
| Pool health | `pool.balance / total_oi_notional` | 1.0 – `pool_health_mult_max` |

### 6.2 Design Properties

- **Pure function:** reads state, returns multipliers. No side effects.
- **Deterministic:** same inputs always produce same outputs.
- **No external dependencies:** all signals from Percolator public fields + pool state.

### 6.3 v2 Extension Point

The `RiskIndex` struct will gain a fourth field:
```rust
pub volatility_multiplier: (u128, u128),  // from off-chain oracle
```

Fed by an off-chain co-processor via the same oracle pattern Percolator uses
for price feeds. The wrapper accepts a `volatility_index` parameter on its
public methods, similar to how Percolator accepts `oracle_price`.

## 7. Wrapper Orchestration

### 7.1 Wrapped Operations

Every public method follows the same pattern:
1. Collect any accrued premiums owed by the account
2. Execute the Percolator operation via its public API
3. Check for deficit, top up pool → Percolator fund if needed

| Operation | Premium Action | Deficit Check |
|-----------|---------------|---------------|
| `deposit` | Collect accrued if position active | No |
| `execute_trade` | 24h commitment upfront + activate accrual | Yes |
| `settle_account` | Collect accrued premium | Yes |
| `withdraw` | Collect accrued + enforce commitment | No |
| `liquidate` | Collect outstanding + deactivate | Yes |
| `keeper_crank` | Batch premium collection on touched accounts | Yes |

### 7.2 24-Hour Commitment

On position open (`execute_trade` where account goes from flat to positioned):

```
premium_rate = compute_premium_per_slot(account, market_state, pool)
commitment = premium_rate × min_commitment_slots

charge_account_fee_not_atomic(idx, commitment, now_slot)
→ pool.balance += premium (accounting)

account_premiums[idx].commitment_end_slot = now_slot + min_commitment_slots
account_premiums[idx].prepaid_premium = commitment
account_premiums[idx].last_premium_slot = now_slot
account_premiums[idx].is_active = true
```

On position close before commitment expires:
- Remaining commitment stays in the pool (not refunded)
- `is_active` set to false

On position close after commitment expires:
- Only accrued premium since last collection is charged
- `is_active` set to false

### 7.3 Per-Slot Premium Accrual

Collected whenever the account is touched (trade, settle, withdraw, crank):

```
slots_elapsed = now_slot - last_premium_slot
if slots_elapsed == 0 || !is_active { return Ok(()); }

current_rate = compute_premium_per_slot(account, market_state, pool)
premium_owed = current_rate × slots_elapsed

// Deduct prepaid first
if prepaid_premium > 0 {
    let from_prepaid = min(premium_owed, prepaid_premium);
    premium_owed -= from_prepaid;
    prepaid_premium -= from_prepaid;
}

// Remaining owed is charged from capital
if premium_owed > 0 {
    charge_account_fee_not_atomic(idx, premium_owed, now_slot)
    → pool.balance += premium (accounting)
}

last_premium_slot = now_slot
```

### 7.4 Error Types

```rust
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
```

## 8. Testing Strategy

### 8.1 Unit Tests (per module)

**`premium.rs`:**
- Leverage multiplier correctness at 1x, 5x, 10x, 25x, 50x, 100x
- Crowding multiplier: balanced → 1.0, one-sided → cap, minority side → 1.0
- Pool health multiplier: healthy → 1.0, depleted → max
- Floor enforcement: dust positions → min_premium_per_slot
- Golden values from Section 4.6

**`pool.rs`:**
- Collect increases balance and total_collected
- Payout decreases balance, increases total_paid_out
- Payout capped at balance (never negative)
- Invariant assertion after every operation

**`risk_index.rs`:**
- Balanced market → all multipliers at floor
- Extreme imbalance → crowding at cap
- Empty vault → OI/vault at cap
- Flat account → no crowding premium

### 8.2 Integration Tests

- Full lifecycle: deposit → trade → accrue → settle → withdraw
- Deficit flow: liquidation → insurance drained → pool detects → top-up
- Commitment enforcement: early close → full 24h charged
- Anti-gaming: N small accounts = 1 large account premium
- Conservation: Percolator's `check_conservation()` holds after every wrapped operation

### 8.3 Fuzz Tests (proptest)

- Random capital, leverage, OI states, pool balances
- Properties: premium >= 0, monotonic with leverage, no overflow, conservation holds
- Sequences of random operations (deposit, trade, liquidate, withdraw)

### 8.4 Numerical Accuracy

- Hardcoded golden values from the walkthrough in Section 4.6
- Rounding direction always conservative (premium rounded up)
- Edge cases: zero position, zero vault, zero pool, u128 boundary values

## 9. Scope

### 9.1 v1 (This Spec)

- Per-slot premium accrual via Percolator's fee mechanism
- 24h upfront commitment on position open
- Configurable leverage exponent
- Three on-chain risk signals (crowding, OI/vault, pool health)
- Separate PremiumPool with full accounting
- Automatic deficit detection and top-up into Percolator's fund
- `no_std` compatible, pure integer math, u256 intermediates

### 9.2 v2 (Deferred)

- Off-chain volatility oracle integration (fourth risk signal)
- Tiered reserve structure (60% instant / 30% yield / 10% reinsurance)
- Reinsurance escrow bucket
- Exit fee during stress events
- Cross-account exposure detection (anti-Sybil behavioral clustering)

## 10. Security Considerations

- **Attack surface:** Wrapper only accesses Percolator's public API. No unsafe
  code. No access to private internals.
- **Premium gaming:** Splitting positions across accounts does not reduce total
  premium — premium is proportional to notional per position, not per account.
- **Overflow:** All intermediate products fit in u256 with >1e31 headroom.
- **Conservation:** Every premium collection and deficit top-up goes through
  Percolator's atomic operations, which enforce `V >= C_tot + I`.
- **Denial of service:** Premium computation is O(1) per account — no loops,
  no iteration over other accounts.
- **Oracle dependency (v1):** None. All signals derived from on-chain state.
