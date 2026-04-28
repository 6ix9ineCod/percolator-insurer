# Simulation & Live Flow Engine — Design Specification

**Date:** 2026-04-28
**Status:** Approved
**Scope:** Historical replay, live feed prototype, parameter optimizer for percolator-insurance

## 1. Problem Statement

The insurance premium pool (percolator-insurance) has tunable parameters —
base_rate, leverage exponent, crowding/OI/pool thresholds — but no empirical
basis for choosing values. We need to:

1. Prove haircuts never trigger under realistic market conditions when premiums
   keep the insurance fund solvent
2. Find the optimal parameter set that maximizes fund health without
   over-charging users
3. Prototype real-time order flow ingestion so premium parameters can
   eventually react to market microstructure before cascading liquidations
   drain the fund

## 2. Architecture

### 2.1 Crate Structure

```
percolator-sim/
  Cargo.toml
  src/
    lib.rs              — re-exports, shared types
    data/
      mod.rs            — data source abstraction
      binance.rs        — Binance archive CSV parser
      tardis.rs         — Tardis.dev order book parser
    feed/
      mod.rs            — live feed abstraction
      binance_ws.rs     — Binance websocket client
      bybit_ws.rs       — Bybit websocket client
      okx_ws.rs         — OKX websocket client
      coinbase_ws.rs    — Coinbase websocket client
    signal/
      mod.rs            — FlowSignal trait + composite scorer
      volume.rs         — volume imbalance detector
      depth.rs          — order book depth thinning detector
      aggression.rs     — trade aggression ratio
    engine/
      mod.rs            — SimEngine harness
      accounts.rs       — 64-account population manager
      clock.rs          — slot/time mapping
    optimizer/
      mod.rs            — Nelder-Mead bounded optimizer
      bounds.rs         — parameter min/max constraints
      rate_limit.rs     — max 10% shift per hour
      objective.rs      — fund surplus with premium budget cap
    metrics/
      mod.rs            — MetricsCollector
      report.rs         — .txt report writer
  bin/
    sim_replay.rs       — historical data replay binary
    sim_live.rs         — real-time exchange feed binary
    sim_optimize.rs     — parameter search binary
```

### 2.2 Dependencies

```toml
[dependencies]
percolator-insurance = { path = "../percolator-insurance" }
percolator = { path = "../", features = ["test"] }
tokio = { version = "1", features = ["full"] }
tungstenite = { version = "0.24", features = ["native-tls"] }
tokio-tungstenite = { version = "0.24", features = ["native-tls"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
csv = "1.3"
chrono = "0.4"
```

### 2.3 Dependency Boundary

percolator-sim depends on percolator-insurance's public API only:
- `InsuredRiskEngine::new`
- `InsuredRiskEngine::deposit`
- `InsuredRiskEngine::execute_trade`
- `InsuredRiskEngine::withdraw`
- `InsuredRiskEngine::liquidate`
- `InsuredRiskEngine::collect_accrued_premium`
- `InsuredRiskEngine::reconcile_pool`
- `InsuredRiskEngine::compute_risk_index`
- `PremiumParams` (all fields)
- `PremiumPool` (balance, total_collected, total_paid_out)

From Percolator directly (via `engine` field):
- `haircut_ratio()` — primary success metric
- `check_conservation()` — invariant verification
- `insurance_fund.balance.get()` — fund health tracking
- `vault.get()` — system leverage
- `accounts[idx]` — position state, capital
- `keeper_crank_not_atomic` — batch operations

## 3. Flow Signal

### 3.1 Signal Components

Three independent detectors, each producing a score from 0 to 100:

**Volume Imbalance:**
```
delta_bid = sum(bid_volume) over window
delta_ask = sum(ask_volume) over window
imbalance = |delta_bid - delta_ask| / (delta_bid + delta_ask)
score = imbalance × 100
```
Rolling windows: 1s, 5s, 30s. Final score = max of all three windows.
When both sides are zero, score = 0.

**Depth Thinning:**
```
depth_now = sum(qty at top N bid levels) + sum(qty at top N ask levels)
depth_prev = same, from previous snapshot
thinning = (depth_prev - depth_now) / depth_prev
score = clamp(thinning × 200, 0, 100)
```
N = 10 levels. Snapshot interval = 100ms (matching exchange update rate).
Negative thinning (depth increasing) → score = 0.

**Trade Aggression:**
```
taker_buy = sum(taker buy volume) over 5s window
taker_sell = sum(taker sell volume) over 5s window
ratio = max(taker_buy, taker_sell) / (taker_buy + taker_sell)
score = (ratio - 0.5) × 200
```
Balanced market (50/50) → score = 0. Fully one-sided → score = 100.

### 3.2 Composite Toxicity Score

```
toxicity = 0.4 × volume_imbalance + 0.3 × depth_thinning + 0.3 × aggression
```

Weights chosen to emphasize volume (earliest signal) while giving depth and
aggression equal weight (confirmation signals).

Output: single u8 value 0–100. Updated every 100ms from live feeds, or per
tick from historical replay.

### 3.3 Signal Usage

The flow toxicity score is logged alongside engine state for correlation
analysis. In v1, it does NOT feed back into premium parameters — it is
observational only. The optimizer uses it post-hoc to evaluate whether
parameter adjustments correlate with toxicity spikes.

Future (v2): toxicity score feeds the volatility_multiplier field in
RiskIndex, creating a real-time premium adjustment loop.

## 4. Engine Harness

### 4.1 SimEngine

Wraps `InsuredRiskEngine` with simulation orchestration:

```rust
pub struct SimEngine {
    pub engine: InsuredRiskEngine,
    pub clock: SlotClock,
    pub accounts: AccountManager,
    pub metrics: MetricsCollector,
}
```

### 4.2 Account Population

64 accounts used directly (Percolator test feature MAX_ACCOUNTS=64).
No bucketing or abstraction. Accounts are assigned positions based on
replay data:

- Accounts 0–59: trading accounts, assigned positions round-robin from
  replay trade events
- Accounts 60–63: reserved for LP / counterparty roles

Account lifecycle per replay event:
1. If account has no position → deposit capital, execute_trade to open
2. If account has position and event is close → execute_trade to flatten
3. Liquidation check after every oracle price update

Capital per account is derived from the trade's implied leverage:
```
capital = notional / desired_leverage
```

### 4.3 Slot/Time Mapping

Percolator uses abstract slots. The sim maps real timestamps to slots:

```
slot = (timestamp_ms - sim_start_ms) / slot_duration_ms
```

Default slot_duration_ms = 400 (matching Solana block time).

The clock advances monotonically. Multiple events at the same timestamp
share a slot. Oracle price updates happen at every slot transition.

### 4.4 Simulation Loop

```
for each event in replay_data:
    advance clock to event.timestamp
    update oracle price
    process event (open/close/modify position)
    run liquidation sweep (check all positioned accounts)
    run keeper_crank if needed
    collect metrics snapshot
    record flow signal score
end

write_report(metrics, output_path)
```

Liquidation sweep after every oracle update ensures cascade behavior
matches what would happen on-chain.

## 5. Optimizer

### 5.1 Algorithm

Nelder-Mead simplex method operating on a bounded parameter space.
Chosen for: no gradient needed, handles noisy objectives, simple to
implement, well-understood convergence.

### 5.2 Parameter Space

Each PremiumParams field has hard-coded min/max bounds:

| Parameter | Min | Max | Unit |
|-----------|-----|-----|------|
| base_rate_per_slot | 10 | 1000 | per PREMIUM_SCALE |
| leverage_exponent_num | 1 | 3 | rational num |
| leverage_exponent_den | 1 | 2 | rational den |
| min_commitment_slots | 54000 | 432000 | slots (6h–48h) |
| crowding_cap | 2000 | 8000 | per MULT_SCALE |
| oi_vault_mult_max | 1500 | 5000 | per MULT_SCALE |
| pool_health_mult_max | 2000 | 10000 | per MULT_SCALE |
| min_premium_per_slot | 1 | 100 | token units |

Threshold ratios (crowding, oi_vault, pool_health) are fixed at the
values from the insurance spec. Only the multiplier magnitudes are tuned.

### 5.3 Rate Limiter

Max 10% parameter shift per hour. Enforced per-parameter:

```
max_delta = current_value × 10 / 100
new_value = clamp(proposed, current - max_delta, current + max_delta)
```

Applied after each optimizer iteration. Both bounds and rate limit are
active simultaneously.

### 5.4 Objective Function

```
objective(params) = fund_surplus / total_notional_traded
    where fund_surplus = fund_balance_end - fund_balance_start

subject to:
    total_premiums_collected / total_notional_traded < budget_cap
```

Default budget_cap = 0.001 (0.1% of notional — competitive with
major perps exchanges).

Infeasible points (budget exceeded) return objective = -infinity.

The optimizer runs the full replay simulation for each parameter
evaluation. One evaluation = one full sim run.

### 5.5 Convergence

Termination criteria:
- Simplex diameter < 1% of parameter range on all dimensions
- Or max 500 iterations
- Or 50 iterations without improvement > 0.1%

## 6. Metrics & Report

### 6.1 MetricsCollector

Samples engine state at configurable intervals (default: every 100 slots).

Tracked values per snapshot:
- `slot`, `timestamp_ms`
- `insurance_fund_balance`
- `pool_balance`, `pool_total_collected`, `pool_total_paid_out`
- `haircut_ratio` (num, den)
- `vault_balance`
- `total_oi_long`, `total_oi_short`
- `active_accounts` (count)
- `flow_toxicity_score`
- `liquidations_this_interval` (count)

### 6.2 Report Format (.txt)

```
══════════════════════════════════════════════════
  PERCOLATOR-SIM REPORT — {scenario_name}
  Generated: {datetime}
  Duration: {slots} slots ({human_duration})
══════════════════════════════════════════════════

─── PARAMETERS ───
  base_rate_per_slot:     {value}
  leverage_exponent:      {num}/{den}
  min_commitment_slots:   {value}
  crowding_cap:           {value}
  oi_vault_mult_max:      {value}
  pool_health_mult_max:   {value}
  min_premium_per_slot:   {value}
  budget_cap:             {value}%

─── FUND HEALTH ───
  Start balance:          {value}
  End balance:            {value}
  Min balance:            {value} (slot {slot})
  Max balance:            {value} (slot {slot})
  Surplus:                {value} ({pct}% of notional)
  Deficit slots:          {count} ({pct}% of duration)
  Haircut activations:    {count}
  Haircut duration:       {slots} slots total

─── PREMIUMS ───
  Total collected:        {value}
  Avg per slot per acct:  {value}
  As % of notional:       {pct}%
  Budget cap:             {budget}%
  Budget status:          UNDER/OVER

─── LIQUIDATIONS ───
  Total count:            {count}
  Capital liquidated:     {value}
  Cascade events:         {count} (>3 liqs within 100 slots)
  Largest cascade:        {count} liqs in {slots} slots

─── FLOW SIGNAL ───
  Avg toxicity:           {value}/100
  Max toxicity:           {value}/100 (slot {slot})
  Time above 70:          {slots} slots ({pct}%)
  Correlation w/ fund
    drawdown:             {pearson_r}

─── VERDICT ───
  {PASS/FAIL}: {summary}
══════════════════════════════════════════════════
```

Verdict logic:
- PASS if: zero haircut activations AND budget under cap
- FAIL if: any haircut activation OR budget over cap
- Summary line explains the result

### 6.3 Output Path

Reports saved to `percolator-sim/output/{scenario}-{timestamp}.txt`.
Optimizer runs save one report per evaluation plus a final summary.

## 7. Data Sources

### 7.1 Historical (sim-replay)

**Binance free archives** (data.binance.vision):
- Aggregated trades CSV: timestamp, price, qty, is_buyer_maker
- Klines CSV: open, high, low, close, volume
- Used for bulk simulation across months of data

**Tardis.dev** (paid, crash events only):
- Full order book snapshots + incremental updates
- Trade-level data with maker/taker labels
- Target events: LUNA crash (May 2022), FTX collapse (Nov 2022),
  Aug 2024 yen carry unwind
- Needed for depth thinning signal (requires L2 book data)

### 7.2 Live (sim-live)

Direct websocket connections to 4 exchanges:
- Binance: wss://fstream.binance.com/ws
- Bybit: wss://stream.bybit.com/v5/public/linear
- OKX: wss://ws.okx.com:8443/ws/v5/public
- Coinbase: wss://advanced-trade-ws.coinbase.com

Streams subscribed:
- Aggregated trades (all 4)
- Order book depth updates, top 20 levels (all 4)

Update latency target: ~100ms per exchange.

### 7.3 Data Parsing

All parsers implement a common trait:

```rust
pub trait DataSource {
    fn next_event(&mut self) -> Option<MarketEvent>;
}

pub enum MarketEvent {
    Trade {
        timestamp_ms: u64,
        price: u64,       // scaled to Percolator oracle format
        qty: u128,        // scaled to POS_SCALE
        is_buy: bool,
    },
    BookUpdate {
        timestamp_ms: u64,
        bids: Vec<(u64, u128)>,  // (price, qty)
        asks: Vec<(u64, u128)>,
    },
}
```

Price scaling: exchange prices (f64 USD) → Percolator oracle price (u64)
via `price_usd × 1_000_000` (POS_SCALE).

## 8. Binaries

### 8.1 sim-replay

```
Usage: sim-replay --data <path> --format <binance|tardis> --params <params.json> --output <path.txt>

Options:
  --data       Path to historical data file or directory
  --format     Data format (binance-trades, binance-klines, tardis-book)
  --params     JSON file with PremiumParams values
  --output     Output report path (default: output/{scenario}-{timestamp}.txt)
  --slots      Max slots to simulate (default: unlimited)
  --accounts   Number of trading accounts (default: 60, max: 60)
```

### 8.2 sim-live

```
Usage: sim-live --exchanges <list> --symbol <BTCUSDT> --params <params.json> --duration <seconds>

Options:
  --exchanges  Comma-separated: binance,bybit,okx,coinbase
  --symbol     Trading pair (default: BTCUSDT)
  --params     JSON file with PremiumParams values
  --duration   How long to run in seconds (default: 3600)
  --output     Output report path
```

Runs the engine in real-time, mapping wall-clock seconds to slots.
Simulated accounts open positions based on observed trade flow direction.

### 8.3 sim-optimize

```
Usage: sim-optimize --data <path> --format <binance|tardis> --budget-cap <pct> --output <path.txt>

Options:
  --data         Historical data for each evaluation run
  --format       Data format
  --budget-cap   Max premiums as % of notional (default: 0.1)
  --max-iter     Max optimizer iterations (default: 500)
  --output       Final report path
  --seed         RNG seed for initial simplex (default: random)
```

Outputs: best parameters found + the simulation report for those parameters.

## 9. Testing Strategy

### 9.1 Unit Tests

- Signal computation: known inputs → expected scores
- Volume imbalance: balanced → 0, fully one-sided → 100
- Depth thinning: no change → 0, 50% withdrawal → 100
- Aggression: 50/50 → 0, 100/0 → 100
- Composite weights sum correctly
- Optimizer bounds enforcement
- Rate limiter: 10% cap per hour

### 9.2 Integration Tests

- Full replay of a small synthetic dataset (100 events)
- Verify: report generated, all sections populated, metrics consistent
- Verify: optimizer finds better params than default after N iterations
- Verify: conservation holds throughout every replay

### 9.3 Smoke Tests

- sim-replay runs to completion on sample Binance CSV
- sim-live connects, receives data, shuts down cleanly after 10s
- Report file is valid and parseable

## 10. Scope

### 10.1 This Spec (v1)

- Historical replay from Binance archives and Tardis crash data
- Live feed prototype (4 exchanges, observation only)
- Flow toxicity score (composite, observational — no feedback loop)
- Nelder-Mead optimizer with bounds + rate limiting
- 64 accounts, no bucketing
- Plain text report output

### 10.2 Deferred (v2)

- Flow toxicity → volatility_multiplier feedback loop (real-time premium adjustment)
- Account bucketing / representative agent replay for >64 positions
- Multi-scenario optimizer (optimize across multiple crash events simultaneously)
- Persistent metrics database (replace .txt with time-series DB)
- Web dashboard for live monitoring
