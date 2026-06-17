# Percolator Insurance — Production Readiness

## Problem

The optimizer overfits to single-day data windows. Parameters that score perfectly on a 6-hour Apr 25 slice produce 24 haircuts on Apr 27 and blow the premium budget by 24x on Apr 26. The leverage exponent collapses to linear (1/1) and min_commitment_slots inflates to 20 hours — both unrealistic for production. The live feed binary has never been tested and contains hardcoded assumptions that will fail against real market data.

## Scope

Three workstreams, executed sequentially:

1. Multi-day optimizer with robust scoring and constrained bounds
2. SimConfig export format for params portability
3. Live feed validation (smoke test + 1-hour run)

## 1. Multi-Day Data Pipeline

### Data Source

Download 7 days of Binance BTCUSDT aggTrades from `https://data.binance.vision/data/futures/um/daily/aggTrades/BTCUSDT/`:

- `BTCUSDT-aggTrades-2026-04-21.zip` through `BTCUSDT-aggTrades-2026-04-27.zip`

Store unzipped CSVs in `percolator/data/`. We already have Apr 25-27; download Apr 21-24.

### Download Validation

After downloading each CSV:
- Verify file size > 0 bytes
- Parse the first 10 rows to confirm the CSV header/structure matches Binance aggTrades format
- Abort the optimizer if any of the 7 days is missing or corrupt

### Optimizer Changes

**New flag:** `--data-dir <path>` (alternative to `--data`). When provided, the optimizer:

1. Reads all `*.csv` files from the directory
2. Sorts filenames lexicographically (date-stamped names sort chronologically)
3. For each evaluation, runs the sim independently on every CSV with the same params
4. Each day starts fresh: same `fund_seed`, same `vault_seed`, same `init_price`
5. Returns **min(day_scores)** as the composite score

**Why min, not mean:** The optimizer must maximize the worst-case day. Arithmetic mean lets good days mask catastrophic failures. Min forces params to survive every regime in the training set. If any single day scores `NEG_INFINITY`, the composite is `NEG_INFINITY`.

**Backward compatibility:** `--data` (single file) continues to work as before. `--data` and `--data-dir` are mutually exclusive; exactly one must be provided.

## 2. Constrained Parameter Bounds

Update `default_param_bounds()` in `optimizer/bounds.rs`:

| Parameter | Current | New | Rationale |
|-----------|---------|-----|-----------|
| `leverage_exponent` | 1.0 – 3.0 | 1.3 – 3.0 | Effective min becomes 1.5 (6/4→3/2) with denom 4. Prevents collapse to linear. |
| `min_commitment_slots` | 54,000 – 432,000 | 100 – 2,700 | 40ms – 18 min at 400ms slots. Current 180K slots = 20 hours is unrealistic. |

All other bounds unchanged.

## 3. Objective Function

No changes to the scoring formula. The existing implementation already:
- Returns `NEG_INFINITY` when premium ratio exceeds budget cap
- Applies `0.5^haircuts` penalty
- Computes `surplus / total_notional` as base score

The multi-day min-scoring layer wraps this without modifying it.

## 4. SimConfig Export Format

After optimization, export a JSON file containing everything needed to reproduce the run:

```json
{
  "premium_params": {
    "base_rate_per_slot": 213,
    "leverage_exponent_num": 3,
    "leverage_exponent_den": 2,
    "min_commitment_slots": 2700,
    "crowding_low_ratio_num": 1500,
    "crowding_low_ratio_den": 1000,
    "crowding_high_ratio_num": 5000,
    "crowding_high_ratio_den": 1000,
    "crowding_cap": 4186,
    "oi_vault_floor_ratio_num": 1,
    "oi_vault_floor_ratio_den": 1,
    "oi_vault_cap_ratio_num": 5,
    "oi_vault_cap_ratio_den": 1,
    "oi_vault_mult_max": 2803,
    "pool_health_low_num": 1,
    "pool_health_low_den": 100,
    "pool_health_high_num": 5,
    "pool_health_high_den": 100,
    "pool_health_mult_max": 2477,
    "min_premium_per_slot": 13
  },
  "fund_seed": 50000000000,
  "budget_cap": 0.1
}
```

### SimConfig struct

```rust
#[derive(Serialize, Deserialize)]
pub struct SimConfig {
    pub premium_params: PremiumParams,
    pub fund_seed: u128,
    pub budget_cap: f64,
}
```

Location: `percolator-sim/src/config.rs`

Both `sim-replay` and `sim-live` accept `--config <path>` as an alternative to `--params`. When `--config` is used, `fund_seed` and `budget_cap` are loaded from the file. CLI flags `--fund-seed` and `--budget-cap` override the config values only when explicitly provided (not when they carry their default values). Use clap's `ArgMatches::value_source()` to distinguish explicit from default.

`--config` and `--params` are mutually exclusive. If neither is provided, built-in defaults are used (same as current behavior).

The optimizer writes `output/sim-config.json` alongside its report.

## 5. Live Feed Fixes

### Dynamic init price

Remove the hardcoded `50_000 * POS_SCALE` from `sim_live.rs`. Instead:

1. Connect to the websocket
2. Wait for the first trade event
3. Use its price as `init_price`
4. Initialize the engine with that price
5. Then enter the main event loop

Timeout after 30 seconds if no trade arrives.

### Fund seed flag

Add `--fund-seed` to `sim-live` CLI args, default `0`. When `--config` is used, the config's `fund_seed` takes precedence (unless `--fund-seed` is explicitly set).

### Test plan

**Phase 1 — Smoke test (10 minutes):**
- Run `sim-live --symbol BTCUSDT --duration 600 --config output/sim-config.json`
- Verify: websocket connects, events parse (event count > 0), engine doesn't panic, report generates, conservation holds

**Phase 2 — Extended run (1 hour, background):**
- Run `sim-live --symbol BTCUSDT --duration 3600 --config output/sim-config.json`
- Compare report metrics against the optimizer's best-day replay
- Check: haircut count, premium ratio vs budget, fund growth trajectory

## 6. File Changes Summary

| File | Change |
|------|--------|
| `percolator-sim/src/optimizer/bounds.rs` | Update leverage_exponent min to 1.3, min_commitment_slots range to 100–2700 |
| `percolator-sim/bin/sim_optimize.rs` | Add `--data-dir` flag, multi-file evaluation with min-scoring, SimConfig JSON export |
| `percolator-sim/bin/sim_live.rs` | Dynamic init price from first trade, add `--fund-seed` and `--config` flags |
| `percolator-sim/bin/sim_replay.rs` | Add `--config` flag support |
| `percolator-sim/src/config.rs` | New file: SimConfig struct with serde |
| `percolator-sim/src/lib.rs` | Add `pub mod config;` |
| `data/` | 4 new CSV files (Apr 21-24) |

## 7. Execution Order

1. Download and validate Apr 21-24 data
2. Update parameter bounds
3. Add SimConfig struct and `--config` support to all binaries
4. Add `--data-dir` and multi-day min-scoring to optimizer
5. Fix sim-live (dynamic price, fund seed, config loading)
6. Run optimizer overnight on 7-day dataset
7. Validate best params via replay on each individual day
8. Smoke test sim-live (10 min)
9. Extended sim-live run (1 hour, background)

## 8. Success Criteria

- Optimizer converges with `leverage_exponent` ≥ 1.5 (3/2) and `min_commitment_slots` ≤ 2700
- Best params produce ≤ 5 haircuts on EVERY individual day (not just average)
- Premium ratio stays under budget cap on EVERY individual day
- sim-live connects, processes > 0 events, generates report without panic
- All existing 464 tests continue to pass
