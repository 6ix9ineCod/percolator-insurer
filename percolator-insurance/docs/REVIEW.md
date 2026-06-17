# Independent Review — `percolator-insurance`

> Commissioned 2026-06-17. Three independent reviewers — one FinTech engineer and
> two insurance professionals (an actuarial/pricing lens and a market/underwriting
> lens) — each read the source directly. The review was **read-only**; no code was
> changed during it. The one **critical** correctness defect they found (the pool
> over-recording premiums) was fixed immediately afterward in commit `34585ad` and
> is now covered by a regression test
> (`tests/integration_tests.rs::test_pool_records_only_actual_collection_when_capital_insufficient`).

## Consensus (TL;DR)

| Lens | Verdict | Headline finding |
|------|---------|------------------|
| FinTech engineering | **NO-GO for production · publish as reference: YES** | Real correctness bug: the pool over-recorded premiums (now **FIXED**). |
| Actuarial / pricing | **DECLINE to certify** | `base_rate` uncalibrated, **no volatility term**, `leverage^1.5` is a guess, and `pool_health` is a pro-cyclical death-spiral accelerant. |
| Market / underwriting | **RESEARCH-REFERENCE-ONLY** | "This is not insurance — it's a dynamic risk-priced protocol fee." Closest live analog: GMX/Hyperliquid/CEX funding skew. |

All three independently concluded the engineering is careful and the crate is
worth publishing **as a research/reference implementation**, but it is **not** a
deployable insurance product and the "insurance" framing overstates what it is.

---

## 1. FinTech Engineering Review

**Verdict:** Clean, well-factored integer-math wrapper with careful overflow
engineering and good unit/property coverage — but built on a Percolator API it
used against its own documented contract, with a defect that caused the pool to
over-report collected premiums. Good as a reference/portfolio piece; not ready
for a live market.

**Strengths**
- Disciplined integer numerics (`#![no_std]`, `#![forbid(unsafe_code)]`, no float). The `isqrt`/`inth_root`/`pow_saturating` trio is more careful than most production trading code; `fuzz_isqrt_correct` proves the floor/ceil bracket.
- Clean separation: pure premium math (`premium.rs`), pure risk multipliers (`risk_index.rs`), pure accounting (`pool.rs`), stateful orchestration (`wrapper.rs`).
- Explicit, enforced pool invariant (`balance + total_paid_out == total_collected`), hammered by a `proptest` op-sequence.

**Risks / findings (ranked)**
1. **[CRITICAL — FIXED in `34585ad`] Pool over-recording.** `charge_account_fee_not_atomic` caps the fee at available capital, routes the shortfall to `fee_credits`, drops any excess, and returns `Ok(())`. The wrapper's `Err(InsufficientBalance)` arm was therefore dead code, and `record_collection(remaining)` over-stated `pool.balance` when `capital < remaining`; `reconcile_pool` then booked phantom payouts. Fix: record the measured insurance-fund delta in both the collection and activation paths.
2. **[HIGH] Recurring premiums use the wrong engine API.** The wrapper drives per-slot accrual through `charge_account_fee_not_atomic`, which the engine docstring explicitly forbids for recurring fees (canonical path: `sync_account_fee_to_slot_not_atomic`). Self-consistent today via the crate's own `last_premium_slot`, but fragile. *(Deferred.)*
3. **[HIGH] None of the wrapper-mandated compliance duties are implemented** — authorization, oracle sourcing/clamping, live-PnL admission warmup, rejecting extraction during oracle divergence. Oracle/funding inputs pass straight through unvalidated. *(Deferred.)*
4. **[MEDIUM] `base_rate` overflow fallback silently drops the factor** (`premium.rs` partial-divide path) — wrong rather than conservatively saturated; untested boundary. *(Deferred.)*
5. **[MEDIUM] Atomicity.** Every op is `_not_atomic` with multi-step mutation and no rollback; `collect_accrued_premium` Results are discarded in deposit/trade/withdraw/liquidate. Relies on the host transaction for atomicity. *(Deferred.)*

**Engineering GO/NO-GO for a live market:** NO-GO — biggest blocker is the unimplemented wrapper compliance contract, atop an unaudited base engine.
**Publish as reference:** YES, conditioned on the now-completed crate README + LICENSE and the bug-#1 fix.

---

## 2. Actuarial / Pricing Review

**Verdict: DECLINE to certify.** The premium has the *shape* of a risk load but
no actuarial anchor to the loss it covers.

The covered loss is a **liquidation gap loss**: a bankrupt position whose margin +
liquidation penalty cannot cover the adverse move, draining the insurance fund
(deficit then socialized via the engine's K-shift). The premium must fund that.

**Flaws (ranked)**
1. **`base_rate` is an unanchored free parameter.** No loss-ratio, expected-claim, IBNR, or ruin-probability link anywhere. The pool is a *ledger*, not a *reserving model*. (The test default `base_rate=100` implies ~788% of notional/yr at 1× — proof it was never calibrated.)
2. **No volatility/gap term.** The loss is driven by oracle jump size vs the maintenance buffer, but all six factors are *state* variables — the σ is missing entirely. Premium is flat in volatility while losses are convex in it; the fund is under-funded exactly going into a vol spike.
3. **`leverage^1.5` is the wrong curvature.** Conditional shortfall severity rises faster than linearly as the buffer collapses toward zero at high leverage; `^1.5` is a mild, sub-quadratic load that under-prices the high-leverage tail. The exponent is exposed as a tunable param — i.e. never derived from a loss curve.
4. **Pro-cyclical `pool_health` multiplier — death-spiral accelerant.** It raises premiums (up to 5×) as the pool depletes, and collection can draw down to an account's full remaining capital — shrinking that account's own liquidation buffer and *manufacturing* the deficits the fund must cover. Charging the most when capital is scarcest is textbook pro-cyclicality. **Single most dangerous design flaw.**
5. **Adverse selection / gaming.** Per-slot accrual + open-time-locked commitment lets a trader prepay cheap at a low-risk-index moment then carry tail exposure; instantaneous leverage is gameable via deposit/withdraw flicker; crowding penalizes the majority side while socialized deficits hit the *minority* side (factor pointed at the wrong cohort).

**Certification:** DECLINE as presented. Path to CERTIFY-WITH-CONDITIONS: anchor `base_rate` to a loss model + ruin target; inject a volatility/gap factor; re-derive the leverage exponent from the buffer curve; cap stress-period collection below the liquidation buffer (counter-cyclical); charge on a TWA to close gaming windows; re-point or drop crowding; add Monte-Carlo stress backtests proving premium income ≥ realized fund draw at the 99.5th percentile.

---

## 3. Market & Underwriting Review

**Verdict: RESEARCH-REFERENCE-ONLY.** It is not insurance — it is a dynamic,
risk-scaled **protocol fee** that funds a socialized backstop. No distinct
policyholder, no defined covered event, no claim payout; the pool only *infers*
consumption from the shared fund draining.

**Closest live analogs:** GMX/GLP dynamic borrow + price-impact fees, and CEX
open-interest funding skew — economic twins already in production. This is a
cleaner, more explicit articulation of that idea, not a novel product.

**Adoption / viability blockers (ranked)**
1. No policyholder / covered event / claim — the value prop to the payer is empty; the biggest premium-payers are uninsured net contributors (severe basis risk).
2. Wraps unsecured `_not_atomic` engine internals with none of the authorization/oracle/divergence hardening the base README mandates.
3. Premium collection competes with margin in the liquidation path — can accelerate the insolvency it nominally buffers.
4. Inferential pool accounting attributes *any* fund decline to "payout" — breaks outside a closed world.
5. ~20 governance parameters with no oracle/keeper/governance plumbing or calibration story.

**Most credible framing:** position it as a **"dynamic risk-based protocol fee
that prices systemic stress into the backstop"** — a research contribution on how
to *size* insurance-fund contributions by leverage, crowding, system leverage,
and reserve depletion. Never market it as "insurance" to users or regulators
(liability without benefit). This framing is now reflected in the crate README.

---

## Deferred findings — roadmap

Tracked here (convert to GitHub Issues with `gh issue create` if desired):

- [x] **CRITICAL** Pool over-records premiums beyond actual collection — *FIXED `34585ad`* (+ regression test).
- [ ] **HIGH** Route recurring accrual through `sync_account_fee_to_slot_not_atomic` (or document why the parallel `last_premium_slot` bookkeeping is double-charge-safe).
- [ ] **HIGH** Implement wrapper compliance duties: authorization hook, oracle staleness/divergence gating, live-PnL admission warmup.
- [ ] **HIGH (pricing)** Anchor `base_rate` to an expected-loss / ruin-probability model; add a loss-ratio concept to the pool.
- [ ] **HIGH (pricing)** Inject a volatility / gap-risk factor into the premium formula.
- [ ] **MEDIUM (pricing)** Re-derive the leverage exponent from the liquidation-buffer curve; surcharge as `L → 1/maintenance_margin`.
- [ ] **MEDIUM (pricing)** Make `pool_health` collection counter-cyclical — cap stress-period charges below the account's liquidation buffer.
- [ ] **MEDIUM** Charge on a TWA of notional/leverage; re-price commitment to the max observed risk index to close gaming windows.
- [ ] **MEDIUM** Re-point the crowding factor at the cohort that absorbs socialized deficits, or drop it.
- [ ] **MEDIUM** Fix the `base_rate` overflow fallback to saturate-or-error; add a boundary test.
- [ ] **LOW** Stop discarding `collect_accrued_premium` Results in the wrapped ops; define a failure policy.
- [ ] **LOW** Add Kani harnesses (pool invariants across collect/consume/reconcile; premium monotonicity & no-panic; `isqrt`/`inth_root` floor correctness) to match the parent's verification bar.
