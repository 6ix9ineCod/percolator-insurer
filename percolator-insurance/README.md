# percolator-insurance

**EDUCATIONAL RESEARCH PROJECT — NOT PRODUCTION READY. NOT AUDITED. Do NOT use with real funds.**

A wrapper crate that adds a **dynamic, risk-priced protocol fee** on top of
[Percolator](https://github.com/aeyakovenko/percolator)'s risk engine. Premiums
are collected per slot from open positions and routed into Percolator's existing
insurance fund, scaled by leverage, open-interest crowding, system leverage, and
fund health.

## ⚠️ This is a fee, not insurance

Despite the crate name, these premiums are **not insurance coverage**. There is
no policyholder, no defined covered event, and no claim payout. A premium-payer
who is liquidated receives nothing back from this pool. What the crate actually
implements is a **risk-based surcharge** that funds the shared solvency buffer —
mechanically a funding-rate-like fee whose size is set by a risk formula.

This is the same economic idea already live at GMX/GLP (dynamic borrow +
price-impact fees), Hyperliquid (HLP), and CEX open-interest funding skew. The
contribution here is an explicit, auditable, pure-integer reference
implementation of *how to size that contribution* from on-chain risk signals.

## What it computes

Per-slot premium for an open position:

```text
premium_per_slot = notional
                 × base_rate
                 × leverage_multiplier      (leverage^1.5, floored at 1.0)
                 × crowding_multiplier       (penalizes the dominant OI side)
                 × oi_vault_multiplier       (rises with total OI / vault TVL)
                 × pool_health_multiplier    (rises as the fund depletes)
                 ÷ PREMIUM_SCALE
```

Ceiling-divided, floored at `min_premium`. All arithmetic is pure integer math
with `u256`-style intermediates — `#![no_std]`, `#![forbid(unsafe_code)]`, no
floating point. The `PremiumPool` is an accounting claim on Percolator's
insurance fund (it holds no segregated assets); it records the amount that
*actually* reached the fund and reconciles against the fund's balance.

**Time-integrated accrual (anti-gaming).** Rather than pricing the whole elapsed
interval at the instantaneous rate seen at collection time (which a trader can
game by timing *when* they are collected from), the account-independent system
factors (`oi_vault × pool_health × volatility`) are integrated over real elapsed
time by a global accumulator (`cum_system_index`, the funding-rate pattern),
advanced by **every** wrapped op and a public `accrue()` for the keeper. A single
account cannot starve the sampler by going quiet — others' activity advances it.
Per-account leverage is charged at the **max of the interval endpoints**
(hardening leverage flicker). Crowding *side* remains point-sampled (documented
residual). See `docs/REVIEW.md` and the spec for the full design.

## Layout

| File | Responsibility |
|------|----------------|
| `src/premium.rs` | Pure premium math: integer `isqrt`/`inth_root`, `leverage_multiplier`, `compute_premium_per_slot`. |
| `src/risk_index.rs` | Pure risk multipliers: crowding, OI/vault system leverage, pool health. |
| `src/pool.rs` | Premium-pool accounting + invariants (`balance + paid_out == collected`). |
| `src/wrapper.rs` | `InsuredRiskEngine`: collect → operate → reconcile around Percolator's API. |

## Status

- 67 tests (unit, integration, property/`proptest`). `no_std`, no `unsafe`.
- Clean `cargo build` / `cargo clippy` for the crate's own sources.

## Known limitations (research caveats)

This crate demonstrates an idea; it is **not** a deployable product. Before it
could wrap a live market it would need, at minimum:

- **The compliance duties Percolator's README assigns to a wrapper** — authorization,
  oracle sourcing/clamping, live-PnL admission warmup, and rejecting
  extraction while raw-oracle vs engine price diverge. None are implemented; the
  engine's `_not_atomic` primitives are called directly.
- **Actuarial calibration.** `base_rate` is a free parameter with no link to an
  expected-loss / ruin-probability target, and the formula carries **no
  volatility/gap term** — yet the covered loss is gap risk. The `leverage^1.5`
  exponent is a placeholder, not fitted to a loss curve.
- **Counter-cyclicality.** The `pool_health` multiplier raises premiums as the
  fund depletes; combined with charging up to an account's remaining capital,
  this can be pro-cyclical under stress. A production design should cap
  stress-period collection below the account's liquidation buffer.

See the in-tree review notes for the full engineering / actuarial / market
assessment that produced this list.

## Build

```bash
cargo test -p percolator-insurance
cargo clippy -p percolator-insurance
```

## License & attribution

Apache-2.0. Built on [Percolator](https://github.com/aeyakovenko/percolator) by
Anatoly Yakovenko (Apache-2.0); this crate is an independent wrapper and is not
affiliated with or endorsed by the upstream author.
