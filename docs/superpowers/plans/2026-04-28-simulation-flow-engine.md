# Percolator Simulation & Live Flow Engine — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a simulation crate that replays historical market data through `InsuredRiskEngine`, computes flow toxicity signals, optimizes premium parameters via Nelder-Mead, and outputs plain-text reports.

**Architecture:** Separate `percolator-sim` crate depending on `percolator-insurance`. Three binaries (replay, live, optimize) share library modules for data parsing, flow signals, engine harness, optimizer, and metrics. 64 Percolator accounts (test feature), no bucketing.

**Tech Stack:** Rust, tokio, tungstenite, serde, csv, chrono. Percolator + percolator-insurance as path dependencies.

---

### Task 1: Crate Scaffold

**Files:**
- Create: `percolator-sim/Cargo.toml`
- Create: `percolator-sim/src/lib.rs`
- Create: `percolator-sim/src/data/mod.rs`
- Create: `percolator-sim/src/feed/mod.rs`
- Create: `percolator-sim/src/signal/mod.rs`
- Create: `percolator-sim/src/engine/mod.rs`
- Create: `percolator-sim/src/optimizer/mod.rs`
- Create: `percolator-sim/src/metrics/mod.rs`
- Create: `percolator-sim/bin/sim_replay.rs`
- Create: `percolator-sim/bin/sim_live.rs`
- Create: `percolator-sim/bin/sim_optimize.rs`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "percolator-sim"
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"

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
clap = { version = "4", features = ["derive"] }

[[bin]]
name = "sim-replay"
path = "bin/sim_replay.rs"

[[bin]]
name = "sim-live"
path = "bin/sim_live.rs"

[[bin]]
name = "sim-optimize"
path = "bin/sim_optimize.rs"
```

- [ ] **Step 2: Create src/lib.rs with shared types and module declarations**

```rust
pub mod data;
pub mod feed;
pub mod signal;
pub mod engine;
pub mod optimizer;
pub mod metrics;

pub use percolator::{MAX_ACCOUNTS, POS_SCALE, MAX_ORACLE_PRICE};
pub use percolator_insurance::{
    InsuredRiskEngine, PremiumParams, PremiumPool, AccountPremiumState,
    PREMIUM_SCALE, LEVERAGE_SCALE, MULT_SCALE, SLOTS_PER_DAY,
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum MarketEvent {
    Trade {
        timestamp_ms: u64,
        price: u64,
        qty: u128,
        is_buy: bool,
    },
    BookUpdate {
        timestamp_ms: u64,
        bids: Vec<(u64, u128)>,
        asks: Vec<(u64, u128)>,
    },
}

pub trait DataSource {
    fn next_event(&mut self) -> Option<MarketEvent>;
}

pub fn price_to_oracle(price_usd: f64) -> u64 {
    (price_usd * POS_SCALE as f64) as u64
}

pub fn qty_to_position(qty: f64) -> u128 {
    (qty * POS_SCALE as f64) as u128
}
```

- [ ] **Step 3: Create stub modules**

Each module `mod.rs` file should be empty for now (just a comment):

`src/data/mod.rs`:
```rust
pub mod binance;
pub mod tardis;
```

`src/feed/mod.rs`:
```rust
pub mod binance_ws;
pub mod bybit_ws;
pub mod okx_ws;
pub mod coinbase_ws;
```

`src/signal/mod.rs`:
```rust
pub mod volume;
pub mod depth;
pub mod aggression;
```

`src/engine/mod.rs`:
```rust
pub mod accounts;
pub mod clock;
```

`src/optimizer/mod.rs`:
```rust
pub mod bounds;
pub mod rate_limit;
pub mod objective;
```

`src/metrics/mod.rs`:
```rust
pub mod report;
```

Create empty stub files for every declared submodule:
- `src/data/binance.rs`, `src/data/tardis.rs`
- `src/feed/binance_ws.rs`, `src/feed/bybit_ws.rs`, `src/feed/okx_ws.rs`, `src/feed/coinbase_ws.rs`
- `src/signal/volume.rs`, `src/signal/depth.rs`, `src/signal/aggression.rs`
- `src/engine/accounts.rs`, `src/engine/clock.rs`
- `src/optimizer/bounds.rs`, `src/optimizer/rate_limit.rs`, `src/optimizer/objective.rs`
- `src/metrics/report.rs`

- [ ] **Step 4: Create binary stubs**

`bin/sim_replay.rs`:
```rust
fn main() {
    println!("sim-replay: not yet implemented");
}
```

`bin/sim_live.rs`:
```rust
fn main() {
    println!("sim-live: not yet implemented");
}
```

`bin/sim_optimize.rs`:
```rust
fn main() {
    println!("sim-optimize: not yet implemented");
}
```

- [ ] **Step 5: Verify crate compiles**

Run: `cd percolator-sim && cargo check`
Expected: compiles with no errors (warnings about unused code are OK)

- [ ] **Step 6: Commit**

```bash
git add percolator-sim/
git commit -m "feat(sim): scaffold percolator-sim crate with module structure"
```

---

### Task 2: Slot Clock

**Files:**
- Modify: `percolator-sim/src/engine/clock.rs`
- Test: inline `#[cfg(test)]` in `clock.rs`

- [ ] **Step 1: Write failing tests**

In `src/engine/clock.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_clock_starts_at_init_slot() {
        let clock = SlotClock::new(1000, 400);
        assert_eq!(clock.current_slot(), 1000);
        assert_eq!(clock.start_timestamp_ms(), 0);
    }

    #[test]
    fn advance_to_same_timestamp_no_change() {
        let mut clock = SlotClock::new(0, 400);
        clock.set_start(5000);
        let advanced = clock.advance_to(5000);
        assert_eq!(advanced, 0);
        assert_eq!(clock.current_slot(), 0);
    }

    #[test]
    fn advance_by_one_slot() {
        let mut clock = SlotClock::new(0, 400);
        clock.set_start(5000);
        let advanced = clock.advance_to(5400);
        assert_eq!(advanced, 1);
        assert_eq!(clock.current_slot(), 1);
    }

    #[test]
    fn advance_by_many_slots() {
        let mut clock = SlotClock::new(0, 400);
        clock.set_start(0);
        let advanced = clock.advance_to(2000);
        assert_eq!(advanced, 5);
        assert_eq!(clock.current_slot(), 5);
    }

    #[test]
    fn advance_partial_slot_no_advance() {
        let mut clock = SlotClock::new(0, 400);
        clock.set_start(0);
        let advanced = clock.advance_to(399);
        assert_eq!(advanced, 0);
        assert_eq!(clock.current_slot(), 0);
    }

    #[test]
    fn advance_is_monotonic() {
        let mut clock = SlotClock::new(10, 400);
        clock.set_start(1000);
        clock.advance_to(1800); // slot 12
        let advanced = clock.advance_to(1400); // earlier timestamp
        assert_eq!(advanced, 0); // no regression
        assert_eq!(clock.current_slot(), 12);
    }

    #[test]
    fn slot_to_timestamp() {
        let clock = SlotClock::new(0, 400);
        assert_eq!(clock.slot_to_ms(0), 0);
        assert_eq!(clock.slot_to_ms(5), 2000);
        assert_eq!(clock.slot_to_ms(100), 40000);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd percolator-sim && cargo test --lib engine::clock`
Expected: FAIL — `SlotClock` not defined

- [ ] **Step 3: Implement SlotClock**

```rust
pub struct SlotClock {
    current: u64,
    start_timestamp_ms: u64,
    slot_duration_ms: u64,
    started: bool,
}

impl SlotClock {
    pub fn new(init_slot: u64, slot_duration_ms: u64) -> Self {
        Self {
            current: init_slot,
            start_timestamp_ms: 0,
            slot_duration_ms,
            started: false,
        }
    }

    pub fn set_start(&mut self, timestamp_ms: u64) {
        self.start_timestamp_ms = timestamp_ms;
        self.started = true;
    }

    pub fn start_timestamp_ms(&self) -> u64 {
        self.start_timestamp_ms
    }

    pub fn current_slot(&self) -> u64 {
        self.current
    }

    pub fn slot_duration_ms(&self) -> u64 {
        self.slot_duration_ms
    }

    pub fn advance_to(&mut self, timestamp_ms: u64) -> u64 {
        if !self.started {
            self.set_start(timestamp_ms);
            return 0;
        }
        if timestamp_ms <= self.start_timestamp_ms {
            return 0;
        }
        let elapsed_ms = timestamp_ms - self.start_timestamp_ms;
        let target_slot = elapsed_ms / self.slot_duration_ms;
        if target_slot <= self.current {
            return 0;
        }
        let advanced = target_slot - self.current;
        self.current = target_slot;
        advanced
    }

    pub fn slot_to_ms(&self, slot: u64) -> u64 {
        slot * self.slot_duration_ms
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd percolator-sim && cargo test --lib engine::clock`
Expected: 7 tests PASS

- [ ] **Step 5: Commit**

```bash
git add percolator-sim/src/engine/clock.rs
git commit -m "feat(sim): implement SlotClock with timestamp-to-slot mapping"
```

---

### Task 3: Flow Signal — Volume Imbalance

**Files:**
- Modify: `percolator-sim/src/signal/volume.rs`
- Test: inline `#[cfg(test)]` in `volume.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_volume_score_zero() {
        let mut vi = VolumeImbalance::new();
        vi.record_trade(1000, 100, true);
        vi.record_trade(1000, 100, false);
        assert_eq!(vi.score(1000), 0);
    }

    #[test]
    fn fully_one_sided_buy_score_100() {
        let mut vi = VolumeImbalance::new();
        vi.record_trade(1000, 100, true);
        assert_eq!(vi.score(1000), 100);
    }

    #[test]
    fn fully_one_sided_sell_score_100() {
        let mut vi = VolumeImbalance::new();
        vi.record_trade(1000, 100, false);
        assert_eq!(vi.score(1000), 100);
    }

    #[test]
    fn no_trades_score_zero() {
        let vi = VolumeImbalance::new();
        assert_eq!(vi.score(5000), 0);
    }

    #[test]
    fn two_to_one_imbalance() {
        let mut vi = VolumeImbalance::new();
        vi.record_trade(1000, 200, true);
        vi.record_trade(1000, 100, false);
        // imbalance = |200-100|/(200+100) = 1/3 ≈ 33
        let s = vi.score(1000);
        assert!(s >= 30 && s <= 36, "expected ~33, got {}", s);
    }

    #[test]
    fn old_trades_expire_from_1s_window() {
        let mut vi = VolumeImbalance::new();
        vi.record_trade(1000, 100, true);
        // 2 seconds later, outside 1s window
        vi.record_trade(3000, 100, false);
        // Only the sell is in the 1s window at t=3000
        let s = vi.score(3000);
        assert_eq!(s, 100);
    }

    #[test]
    fn max_of_windows_used() {
        let mut vi = VolumeImbalance::new();
        // Old trade (in 30s window, not 1s)
        vi.record_trade(1000, 500, true);
        // Recent trade (in all windows)
        vi.record_trade(29000, 100, false);
        // 1s window at t=29000: only 100 sell → score 100
        // 5s window: only 100 sell → score 100
        // 30s window: 500 buy + 100 sell → |400|/600 = 66
        // max = 100
        let s = vi.score(29000);
        assert_eq!(s, 100);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd percolator-sim && cargo test --lib signal::volume`
Expected: FAIL — `VolumeImbalance` not defined

- [ ] **Step 3: Implement VolumeImbalance**

```rust
struct TradeRecord {
    timestamp_ms: u64,
    qty: u64,
    is_buy: bool,
}

pub struct VolumeImbalance {
    trades: Vec<TradeRecord>,
}

const WINDOW_1S: u64 = 1_000;
const WINDOW_5S: u64 = 5_000;
const WINDOW_30S: u64 = 30_000;

impl VolumeImbalance {
    pub fn new() -> Self {
        Self { trades: Vec::new() }
    }

    pub fn record_trade(&mut self, timestamp_ms: u64, qty: u64, is_buy: bool) {
        self.trades.push(TradeRecord { timestamp_ms, qty, is_buy });
    }

    pub fn score(&self, now_ms: u64) -> u8 {
        let s1 = self.window_score(now_ms, WINDOW_1S);
        let s5 = self.window_score(now_ms, WINDOW_5S);
        let s30 = self.window_score(now_ms, WINDOW_30S);
        s1.max(s5).max(s30)
    }

    fn window_score(&self, now_ms: u64, window_ms: u64) -> u8 {
        let cutoff = now_ms.saturating_sub(window_ms);
        let mut buy_vol: u64 = 0;
        let mut sell_vol: u64 = 0;
        for t in &self.trades {
            if t.timestamp_ms >= cutoff && t.timestamp_ms <= now_ms {
                if t.is_buy {
                    buy_vol = buy_vol.saturating_add(t.qty);
                } else {
                    sell_vol = sell_vol.saturating_add(t.qty);
                }
            }
        }
        let total = buy_vol.saturating_add(sell_vol);
        if total == 0 {
            return 0;
        }
        let diff = if buy_vol > sell_vol {
            buy_vol - sell_vol
        } else {
            sell_vol - buy_vol
        };
        ((diff as u128 * 100) / total as u128) as u8
    }

    pub fn gc(&mut self, now_ms: u64) {
        let cutoff = now_ms.saturating_sub(WINDOW_30S);
        self.trades.retain(|t| t.timestamp_ms >= cutoff);
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd percolator-sim && cargo test --lib signal::volume`
Expected: 7 tests PASS

- [ ] **Step 5: Commit**

```bash
git add percolator-sim/src/signal/volume.rs
git commit -m "feat(sim): implement VolumeImbalance flow signal with multi-window scoring"
```

---

### Task 4: Flow Signal — Depth Thinning

**Files:**
- Modify: `percolator-sim/src/signal/depth.rs`
- Test: inline `#[cfg(test)]` in `depth.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_previous_snapshot_score_zero() {
        let dt = DepthThinning::new(10);
        assert_eq!(dt.score(), 0);
    }

    #[test]
    fn no_change_score_zero() {
        let mut dt = DepthThinning::new(10);
        let levels = vec![(100, 50), (99, 30), (98, 20)];
        dt.update(&levels, &levels);
        assert_eq!(dt.score(), 0);
    }

    #[test]
    fn fifty_percent_thinning() {
        let mut dt = DepthThinning::new(10);
        let bids = vec![(100, 100)];
        let asks = vec![(101, 100)];
        dt.update(&bids, &asks);
        // depth_prev = 200, now halve it
        let bids2 = vec![(100, 50)];
        let asks2 = vec![(101, 50)];
        dt.update(&bids2, &asks2);
        // thinning = (200 - 100) / 200 = 0.5, score = clamp(0.5 * 200, 0, 100) = 100
        assert_eq!(dt.score(), 100);
    }

    #[test]
    fn twenty_five_percent_thinning() {
        let mut dt = DepthThinning::new(10);
        let bids = vec![(100, 100)];
        let asks = vec![(101, 100)];
        dt.update(&bids, &asks);
        let bids2 = vec![(100, 75)];
        let asks2 = vec![(101, 75)];
        dt.update(&bids2, &asks2);
        // thinning = (200-150)/200 = 0.25, score = 0.25 * 200 = 50
        assert_eq!(dt.score(), 50);
    }

    #[test]
    fn depth_increasing_score_zero() {
        let mut dt = DepthThinning::new(10);
        let bids = vec![(100, 50)];
        let asks = vec![(101, 50)];
        dt.update(&bids, &asks);
        let bids2 = vec![(100, 100)];
        let asks2 = vec![(101, 100)];
        dt.update(&bids2, &asks2);
        assert_eq!(dt.score(), 0);
    }

    #[test]
    fn only_top_n_levels_counted() {
        let mut dt = DepthThinning::new(2);
        let bids = vec![(100, 50), (99, 30), (98, 9999)];
        let asks = vec![(101, 50), (102, 30), (103, 9999)];
        dt.update(&bids, &asks);
        // top 2 bids: 50+30=80, top 2 asks: 50+30=80, total=160
        let bids2 = vec![(100, 25), (99, 15), (98, 9999)];
        let asks2 = vec![(101, 25), (102, 15), (103, 9999)];
        dt.update(&bids2, &asks2);
        // new total = 80, thinning = (160-80)/160 = 0.5, score = 100
        assert_eq!(dt.score(), 100);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd percolator-sim && cargo test --lib signal::depth`
Expected: FAIL — `DepthThinning` not defined

- [ ] **Step 3: Implement DepthThinning**

```rust
pub struct DepthThinning {
    top_n: usize,
    prev_depth: u64,
    curr_depth: u64,
    has_prev: bool,
}

impl DepthThinning {
    pub fn new(top_n: usize) -> Self {
        Self {
            top_n,
            prev_depth: 0,
            curr_depth: 0,
            has_prev: false,
        }
    }

    pub fn update(&mut self, bids: &[(u64, u64)], asks: &[(u64, u64)]) {
        let bid_sum: u64 = bids.iter().take(self.top_n).map(|(_, q)| q).sum();
        let ask_sum: u64 = asks.iter().take(self.top_n).map(|(_, q)| q).sum();
        let total = bid_sum.saturating_add(ask_sum);

        if self.has_prev {
            self.prev_depth = self.curr_depth;
        }
        self.curr_depth = total;
        self.has_prev = true;
    }

    pub fn score(&self) -> u8 {
        if !self.has_prev || self.prev_depth == 0 {
            return 0;
        }
        if self.curr_depth >= self.prev_depth {
            return 0;
        }
        let diff = self.prev_depth - self.curr_depth;
        let raw = (diff as u128 * 200) / self.prev_depth as u128;
        raw.min(100) as u8
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd percolator-sim && cargo test --lib signal::depth`
Expected: 6 tests PASS

- [ ] **Step 5: Commit**

```bash
git add percolator-sim/src/signal/depth.rs
git commit -m "feat(sim): implement DepthThinning flow signal"
```

---

### Task 5: Flow Signal — Trade Aggression + Composite

**Files:**
- Modify: `percolator-sim/src/signal/aggression.rs`
- Modify: `percolator-sim/src/signal/mod.rs`
- Test: inline `#[cfg(test)]` in both files

- [ ] **Step 1: Write failing tests for TradeAggression**

In `src/signal/aggression.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_aggression_score_zero() {
        let mut ta = TradeAggression::new();
        ta.record(1000, 100, true);
        ta.record(1000, 100, false);
        assert_eq!(ta.score(1000), 0);
    }

    #[test]
    fn fully_buy_aggressive_score_100() {
        let mut ta = TradeAggression::new();
        ta.record(1000, 100, true);
        assert_eq!(ta.score(1000), 100);
    }

    #[test]
    fn fully_sell_aggressive_score_100() {
        let mut ta = TradeAggression::new();
        ta.record(1000, 100, false);
        assert_eq!(ta.score(1000), 100);
    }

    #[test]
    fn no_trades_score_zero() {
        let ta = TradeAggression::new();
        assert_eq!(ta.score(5000), 0);
    }

    #[test]
    fn seventy_five_twenty_five_split() {
        let mut ta = TradeAggression::new();
        ta.record(1000, 75, true);
        ta.record(1000, 25, false);
        // ratio = 75/100 = 0.75, score = (0.75 - 0.5) * 200 = 50
        assert_eq!(ta.score(1000), 50);
    }

    #[test]
    fn old_trades_expire() {
        let mut ta = TradeAggression::new();
        ta.record(1000, 100, true);
        ta.record(7000, 50, false);
        // At t=7000, 5s window starts at 2000, so first trade is expired
        let s = ta.score(7000);
        assert_eq!(s, 100);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd percolator-sim && cargo test --lib signal::aggression`
Expected: FAIL — `TradeAggression` not defined

- [ ] **Step 3: Implement TradeAggression**

```rust
const WINDOW_5S: u64 = 5_000;

struct AggressionRecord {
    timestamp_ms: u64,
    qty: u64,
    is_buy: bool,
}

pub struct TradeAggression {
    trades: Vec<AggressionRecord>,
}

impl TradeAggression {
    pub fn new() -> Self {
        Self { trades: Vec::new() }
    }

    pub fn record(&mut self, timestamp_ms: u64, qty: u64, is_buy: bool) {
        self.trades.push(AggressionRecord { timestamp_ms, qty, is_buy });
    }

    pub fn score(&self, now_ms: u64) -> u8 {
        let cutoff = now_ms.saturating_sub(WINDOW_5S);
        let mut buy_vol: u64 = 0;
        let mut sell_vol: u64 = 0;
        for t in &self.trades {
            if t.timestamp_ms >= cutoff && t.timestamp_ms <= now_ms {
                if t.is_buy {
                    buy_vol = buy_vol.saturating_add(t.qty);
                } else {
                    sell_vol = sell_vol.saturating_add(t.qty);
                }
            }
        }
        let total = buy_vol.saturating_add(sell_vol);
        if total == 0 {
            return 0;
        }
        let dominant = buy_vol.max(sell_vol);
        let ratio_x1000 = (dominant as u128 * 1000) / total as u128;
        let score = (ratio_x1000 as i64 - 500) * 200 / 1000;
        (score.max(0) as u8).min(100)
    }

    pub fn gc(&mut self, now_ms: u64) {
        let cutoff = now_ms.saturating_sub(WINDOW_5S);
        self.trades.retain(|t| t.timestamp_ms >= cutoff);
    }
}
```

- [ ] **Step 4: Run aggression tests**

Run: `cd percolator-sim && cargo test --lib signal::aggression`
Expected: 6 tests PASS

- [ ] **Step 5: Write failing tests for FlowSignal composite in signal/mod.rs**

```rust
pub use volume::VolumeImbalance;
pub use depth::DepthThinning;
pub use aggression::TradeAggression;

pub struct FlowSignal {
    pub volume: VolumeImbalance,
    pub depth: DepthThinning,
    pub aggression: TradeAggression,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_all_zero() {
        let fs = FlowSignal::new();
        assert_eq!(fs.toxicity(1000), 0);
    }

    #[test]
    fn composite_weighted_correctly() {
        // volume=100, depth=0, aggression=0 → 0.4*100 + 0.3*0 + 0.3*0 = 40
        let mut fs = FlowSignal::new();
        fs.volume.record_trade(1000, 100, true);
        // depth stays 0 (no updates), aggression stays 0
        let t = fs.toxicity(1000);
        assert_eq!(t, 40);
    }

    #[test]
    fn composite_all_max() {
        let mut fs = FlowSignal::new();
        // volume: fully one-sided
        fs.volume.record_trade(1000, 100, true);
        // aggression: fully one-sided
        fs.aggression.record(1000, 100, true);
        // depth: 50% thinning → score 100
        let bids = vec![(100, 100)];
        let asks = vec![(101, 100)];
        fs.depth.update(&bids, &asks);
        let bids2 = vec![(100, 50)];
        let asks2 = vec![(101, 50)];
        fs.depth.update(&bids2, &asks2);
        // 0.4*100 + 0.3*100 + 0.3*100 = 100
        assert_eq!(fs.toxicity(1000), 100);
    }
}
```

- [ ] **Step 6: Implement FlowSignal composite**

Add to `src/signal/mod.rs`:

```rust
pub mod volume;
pub mod depth;
pub mod aggression;

pub use volume::VolumeImbalance;
pub use depth::DepthThinning;
pub use aggression::TradeAggression;

pub struct FlowSignal {
    pub volume: VolumeImbalance,
    pub depth: DepthThinning,
    pub aggression: TradeAggression,
}

impl FlowSignal {
    pub fn new() -> Self {
        Self {
            volume: VolumeImbalance::new(),
            depth: DepthThinning::new(10),
            aggression: TradeAggression::new(),
        }
    }

    pub fn toxicity(&self, now_ms: u64) -> u8 {
        let v = self.volume.score(now_ms) as u32;
        let d = self.depth.score() as u32;
        let a = self.aggression.score(now_ms) as u32;
        let composite = (v * 40 + d * 30 + a * 30) / 100;
        composite.min(100) as u8
    }

    pub fn gc(&mut self, now_ms: u64) {
        self.volume.gc(now_ms);
        self.aggression.gc(now_ms);
    }
}
```

- [ ] **Step 7: Run all signal tests**

Run: `cd percolator-sim && cargo test --lib signal`
Expected: 16 tests PASS (7 volume + 6 depth + 6 aggression + 3 composite... adjust if counts differ based on exact test layout)

- [ ] **Step 8: Commit**

```bash
git add percolator-sim/src/signal/
git commit -m "feat(sim): implement TradeAggression + FlowSignal composite toxicity scorer"
```

---

### Task 6: Metrics Collector + Report Writer

**Files:**
- Modify: `percolator-sim/src/metrics/mod.rs`
- Modify: `percolator-sim/src/metrics/report.rs`
- Test: inline `#[cfg(test)]` in both files

- [ ] **Step 1: Write failing tests**

In `src/metrics/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_collector_empty() {
        let mc = MetricsCollector::new(100);
        assert_eq!(mc.snapshots.len(), 0);
        assert_eq!(mc.liquidation_count, 0);
    }

    #[test]
    fn record_snapshot() {
        let mut mc = MetricsCollector::new(100);
        mc.record(Snapshot {
            slot: 100,
            timestamp_ms: 40000,
            insurance_fund_balance: 5000,
            pool_balance: 1000,
            pool_total_collected: 1200,
            pool_total_paid_out: 200,
            haircut_num: 1,
            haircut_den: 1,
            vault_balance: 100000,
            total_oi_long: 50000,
            total_oi_short: 45000,
            active_accounts: 10,
            flow_toxicity: 35,
        });
        assert_eq!(mc.snapshots.len(), 1);
    }

    #[test]
    fn record_liquidation() {
        let mut mc = MetricsCollector::new(100);
        mc.record_liquidation(50, 10000);
        mc.record_liquidation(55, 5000);
        assert_eq!(mc.liquidation_count, 2);
        assert_eq!(mc.capital_liquidated, 15000);
    }

    #[test]
    fn cascade_detection() {
        let mut mc = MetricsCollector::new(100);
        mc.record_liquidation(100, 1000);
        mc.record_liquidation(120, 1000);
        mc.record_liquidation(150, 1000);
        mc.record_liquidation(180, 1000);
        let cascades = mc.count_cascades(100);
        assert_eq!(cascades.0, 1); // 1 cascade
        assert_eq!(cascades.1, 4); // largest = 4 liqs
    }

    #[test]
    fn no_cascade_when_spread_out() {
        let mut mc = MetricsCollector::new(100);
        mc.record_liquidation(100, 1000);
        mc.record_liquidation(300, 1000);
        mc.record_liquidation(500, 1000);
        let cascades = mc.count_cascades(100);
        assert_eq!(cascades.0, 0); // no cascade (never >3 within 100 slots)
    }

    #[test]
    fn haircut_count() {
        let mut mc = MetricsCollector::new(100);
        mc.record(Snapshot {
            slot: 0, timestamp_ms: 0, insurance_fund_balance: 1000,
            pool_balance: 500, pool_total_collected: 500, pool_total_paid_out: 0,
            haircut_num: 1, haircut_den: 1, vault_balance: 10000,
            total_oi_long: 5000, total_oi_short: 5000, active_accounts: 5,
            flow_toxicity: 0,
        });
        mc.record(Snapshot {
            slot: 100, timestamp_ms: 40000, insurance_fund_balance: 0,
            pool_balance: 0, pool_total_collected: 500, pool_total_paid_out: 500,
            haircut_num: 9, haircut_den: 10, vault_balance: 10000,
            total_oi_long: 5000, total_oi_short: 5000, active_accounts: 5,
            flow_toxicity: 80,
        });
        mc.record(Snapshot {
            slot: 200, timestamp_ms: 80000, insurance_fund_balance: 100,
            pool_balance: 100, pool_total_collected: 600, pool_total_paid_out: 500,
            haircut_num: 1, haircut_den: 1, vault_balance: 10000,
            total_oi_long: 5000, total_oi_short: 5000, active_accounts: 5,
            flow_toxicity: 20,
        });
        assert_eq!(mc.haircut_activations(), 1);
        assert_eq!(mc.haircut_slots(), 1); // only snapshot at slot 100
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd percolator-sim && cargo test --lib metrics`
Expected: FAIL — types not defined

- [ ] **Step 3: Implement MetricsCollector**

In `src/metrics/mod.rs`:

```rust
pub mod report;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub slot: u64,
    pub timestamp_ms: u64,
    pub insurance_fund_balance: u128,
    pub pool_balance: u128,
    pub pool_total_collected: u128,
    pub pool_total_paid_out: u128,
    pub haircut_num: u128,
    pub haircut_den: u128,
    pub vault_balance: u128,
    pub total_oi_long: u128,
    pub total_oi_short: u128,
    pub active_accounts: u32,
    pub flow_toxicity: u8,
}

#[derive(Clone, Debug)]
struct LiquidationEvent {
    slot: u64,
    capital: u128,
}

pub struct MetricsCollector {
    pub snapshots: Vec<Snapshot>,
    pub sample_interval: u64,
    pub liquidation_count: u64,
    pub capital_liquidated: u128,
    pub total_notional_traded: u128,
    liquidations: Vec<LiquidationEvent>,
}

impl MetricsCollector {
    pub fn new(sample_interval: u64) -> Self {
        Self {
            snapshots: Vec::new(),
            sample_interval,
            liquidation_count: 0,
            capital_liquidated: 0,
            total_notional_traded: 0,
            liquidations: Vec::new(),
        }
    }

    pub fn record(&mut self, snapshot: Snapshot) {
        self.snapshots.push(snapshot);
    }

    pub fn record_liquidation(&mut self, slot: u64, capital: u128) {
        self.liquidation_count += 1;
        self.capital_liquidated = self.capital_liquidated.saturating_add(capital);
        self.liquidations.push(LiquidationEvent { slot, capital });
    }

    pub fn record_trade_notional(&mut self, notional: u128) {
        self.total_notional_traded = self.total_notional_traded.saturating_add(notional);
    }

    pub fn haircut_activations(&self) -> u64 {
        let mut count = 0u64;
        let mut was_active = false;
        for s in &self.snapshots {
            let active = s.haircut_num < s.haircut_den;
            if active && !was_active {
                count += 1;
            }
            was_active = active;
        }
        count
    }

    pub fn haircut_slots(&self) -> u64 {
        self.snapshots.iter().filter(|s| s.haircut_num < s.haircut_den).count() as u64
    }

    pub fn count_cascades(&self, window_slots: u64) -> (u64, u64) {
        if self.liquidations.len() < 4 {
            return (0, 0);
        }
        let mut cascade_count = 0u64;
        let mut largest = 0u64;
        let liqs = &self.liquidations;
        let mut i = 0;
        while i < liqs.len() {
            let start_slot = liqs[i].slot;
            let mut j = i + 1;
            while j < liqs.len() && liqs[j].slot <= start_slot + window_slots {
                j += 1;
            }
            let group_size = (j - i) as u64;
            if group_size > 3 {
                cascade_count += 1;
                if group_size > largest {
                    largest = group_size;
                }
            }
            i = j;
        }
        (cascade_count, largest)
    }

    pub fn fund_min(&self) -> (u128, u64) {
        self.snapshots.iter()
            .map(|s| (s.insurance_fund_balance, s.slot))
            .min_by_key(|&(b, _)| b)
            .unwrap_or((0, 0))
    }

    pub fn fund_max(&self) -> (u128, u64) {
        self.snapshots.iter()
            .map(|s| (s.insurance_fund_balance, s.slot))
            .max_by_key(|&(b, _)| b)
            .unwrap_or((0, 0))
    }

    pub fn deficit_slots(&self) -> u64 {
        self.snapshots.iter().filter(|s| s.insurance_fund_balance == 0).count() as u64
    }

    pub fn avg_toxicity(&self) -> u8 {
        if self.snapshots.is_empty() {
            return 0;
        }
        let sum: u64 = self.snapshots.iter().map(|s| s.flow_toxicity as u64).sum();
        (sum / self.snapshots.len() as u64) as u8
    }

    pub fn max_toxicity(&self) -> (u8, u64) {
        self.snapshots.iter()
            .map(|s| (s.flow_toxicity, s.slot))
            .max_by_key(|&(t, _)| t)
            .unwrap_or((0, 0))
    }

    pub fn toxicity_above_threshold(&self, threshold: u8) -> u64 {
        self.snapshots.iter().filter(|s| s.flow_toxicity > threshold).count() as u64
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd percolator-sim && cargo test --lib metrics`
Expected: 7 tests PASS

- [ ] **Step 5: Implement report writer**

In `src/metrics/report.rs`:

```rust
use super::MetricsCollector;
use crate::PremiumParams;
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::fs;
use std::path::Path;

pub struct ReportConfig {
    pub scenario_name: String,
    pub params: PremiumParams,
    pub budget_cap_pct: f64,
    pub fund_start: u128,
    pub fund_end: u128,
    pub total_slots: u64,
    pub slot_duration_ms: u64,
}

pub fn generate_report(metrics: &MetricsCollector, config: &ReportConfig) -> String {
    let mut out = String::new();
    let duration_s = config.total_slots * config.slot_duration_ms / 1000;
    let hours = duration_s / 3600;
    let minutes = (duration_s % 3600) / 60;
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");

    let (fund_min, fund_min_slot) = metrics.fund_min();
    let (fund_max, fund_max_slot) = metrics.fund_max();
    let surplus = if config.fund_end >= config.fund_start {
        config.fund_end - config.fund_start
    } else {
        0
    };
    let deficit_slots = metrics.deficit_slots();
    let haircut_acts = metrics.haircut_activations();
    let haircut_dur = metrics.haircut_slots();

    let last_snap = metrics.snapshots.last();
    let pool_collected = last_snap.map(|s| s.pool_total_collected).unwrap_or(0);

    let notional = metrics.total_notional_traded;
    let premium_pct = if notional > 0 {
        (pool_collected as f64 / notional as f64) * 100.0
    } else {
        0.0
    };
    let surplus_pct = if notional > 0 {
        (surplus as f64 / notional as f64) * 100.0
    } else {
        0.0
    };

    let budget_status = if premium_pct <= config.budget_cap_pct { "UNDER" } else { "OVER" };

    let (cascade_count, largest_cascade) = metrics.count_cascades(100);
    let avg_tox = metrics.avg_toxicity();
    let (max_tox, max_tox_slot) = metrics.max_toxicity();
    let tox_above_70 = metrics.toxicity_above_threshold(70);
    let tox_above_pct = if !metrics.snapshots.is_empty() {
        (tox_above_70 as f64 / metrics.snapshots.len() as f64) * 100.0
    } else {
        0.0
    };
    let deficit_pct = if config.total_slots > 0 {
        (deficit_slots as f64 / (config.total_slots as f64 / metrics.sample_interval as f64)) * 100.0
    } else {
        0.0
    };

    let active_accts = last_snap.map(|s| s.active_accounts).unwrap_or(0);
    let avg_per_slot = if active_accts > 0 && config.total_slots > 0 {
        pool_collected / (config.total_slots as u128 * active_accts as u128)
    } else {
        0
    };

    let verdict = if haircut_acts == 0 && premium_pct <= config.budget_cap_pct {
        "PASS"
    } else {
        "FAIL"
    };
    let verdict_reason = if haircut_acts > 0 {
        format!("{} haircut activation(s) detected", haircut_acts)
    } else if premium_pct > config.budget_cap_pct {
        format!("premium budget exceeded ({:.4}% > {:.4}%)", premium_pct, config.budget_cap_pct)
    } else {
        "no haircut activations, premiums within budget".to_string()
    };

    writeln!(out, "══════════════════════════════════════════════════").ok();
    writeln!(out, "  PERCOLATOR-SIM REPORT — {}", config.scenario_name).ok();
    writeln!(out, "  Generated: {}", now).ok();
    writeln!(out, "  Duration: {} slots ({}h {}m)", config.total_slots, hours, minutes).ok();
    writeln!(out, "══════════════════════════════════════════════════").ok();
    writeln!(out).ok();
    writeln!(out, "─── PARAMETERS ───").ok();
    writeln!(out, "  base_rate_per_slot:     {}", config.params.base_rate_per_slot).ok();
    writeln!(out, "  leverage_exponent:      {}/{}", config.params.leverage_exponent_num, config.params.leverage_exponent_den).ok();
    writeln!(out, "  min_commitment_slots:   {}", config.params.min_commitment_slots).ok();
    writeln!(out, "  crowding_cap:           {}", config.params.crowding_cap).ok();
    writeln!(out, "  oi_vault_mult_max:      {}", config.params.oi_vault_mult_max).ok();
    writeln!(out, "  pool_health_mult_max:   {}", config.params.pool_health_mult_max).ok();
    writeln!(out, "  min_premium_per_slot:   {}", config.params.min_premium_per_slot).ok();
    writeln!(out, "  budget_cap:             {:.4}%", config.budget_cap_pct).ok();
    writeln!(out).ok();
    writeln!(out, "─── FUND HEALTH ───").ok();
    writeln!(out, "  Start balance:          {}", config.fund_start).ok();
    writeln!(out, "  End balance:            {}", config.fund_end).ok();
    writeln!(out, "  Min balance:            {} (slot {})", fund_min, fund_min_slot).ok();
    writeln!(out, "  Max balance:            {} (slot {})", fund_max, fund_max_slot).ok();
    writeln!(out, "  Surplus:                {} ({:.4}% of notional)", surplus, surplus_pct).ok();
    writeln!(out, "  Deficit slots:          {} ({:.1}% of duration)", deficit_slots, deficit_pct).ok();
    writeln!(out, "  Haircut activations:    {}", haircut_acts).ok();
    writeln!(out, "  Haircut duration:       {} slots total", haircut_dur).ok();
    writeln!(out).ok();
    writeln!(out, "─── PREMIUMS ───").ok();
    writeln!(out, "  Total collected:        {}", pool_collected).ok();
    writeln!(out, "  Avg per slot per acct:  {}", avg_per_slot).ok();
    writeln!(out, "  As % of notional:       {:.4}%", premium_pct).ok();
    writeln!(out, "  Budget cap:             {:.4}%", config.budget_cap_pct).ok();
    writeln!(out, "  Budget status:          {}", budget_status).ok();
    writeln!(out).ok();
    writeln!(out, "─── LIQUIDATIONS ───").ok();
    writeln!(out, "  Total count:            {}", metrics.liquidation_count).ok();
    writeln!(out, "  Capital liquidated:     {}", metrics.capital_liquidated).ok();
    writeln!(out, "  Cascade events:         {} (>3 liqs within 100 slots)", cascade_count).ok();
    writeln!(out, "  Largest cascade:        {} liqs", largest_cascade).ok();
    writeln!(out).ok();
    writeln!(out, "─── FLOW SIGNAL ───").ok();
    writeln!(out, "  Avg toxicity:           {}/100", avg_tox).ok();
    writeln!(out, "  Max toxicity:           {}/100 (slot {})", max_tox, max_tox_slot).ok();
    writeln!(out, "  Time above 70:          {} slots ({:.1}%)", tox_above_70, tox_above_pct).ok();
    writeln!(out).ok();
    writeln!(out, "─── VERDICT ───").ok();
    writeln!(out, "  {}: {}", verdict, verdict_reason).ok();
    writeln!(out, "══════════════════════════════════════════════════").ok();

    out
}

pub fn write_report(report: &str, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::File::create(path)?;
    f.write_all(report.as_bytes())
}
```

- [ ] **Step 6: Run all metrics tests**

Run: `cd percolator-sim && cargo test --lib metrics`
Expected: 7 tests PASS

- [ ] **Step 7: Commit**

```bash
git add percolator-sim/src/metrics/
git commit -m "feat(sim): implement MetricsCollector and .txt report writer"
```

---

### Task 7: Binance CSV Data Parser

**Files:**
- Modify: `percolator-sim/src/data/binance.rs`
- Modify: `percolator-sim/src/data/mod.rs`
- Test: inline `#[cfg(test)]` in `binance.rs`

- [ ] **Step 1: Write failing tests**

In `src/data/binance.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::MarketEvent;
    use std::io::Write;

    fn make_csv(rows: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{}", rows).unwrap();
        f
    }

    #[test]
    fn parse_single_trade() {
        // Binance aggTrades CSV: id,price,qty,first_trade_id,last_trade_id,timestamp,is_buyer_maker,is_best_match
        let csv = "123,50000.50,0.001,100,100,1700000000000,false,true\n";
        let f = make_csv(csv);
        let mut src = BinanceTradeSource::from_path(f.path()).unwrap();
        let event = src.next_event().unwrap();
        match event {
            MarketEvent::Trade { timestamp_ms, price, qty, is_buy } => {
                assert_eq!(timestamp_ms, 1700000000000);
                assert_eq!(price, 50000500000); // 50000.50 * 1_000_000
                assert!(qty > 0);
                assert_eq!(is_buy, true); // is_buyer_maker=false → taker is buyer
            }
            _ => panic!("expected Trade"),
        }
    }

    #[test]
    fn parse_multiple_trades() {
        let csv = "1,50000.0,1.0,1,1,1000,false,true\n2,50001.0,2.0,2,2,2000,true,true\n";
        let f = make_csv(csv);
        let mut src = BinanceTradeSource::from_path(f.path()).unwrap();
        assert!(src.next_event().is_some());
        assert!(src.next_event().is_some());
        assert!(src.next_event().is_none());
    }

    #[test]
    fn empty_file() {
        let f = make_csv("");
        let mut src = BinanceTradeSource::from_path(f.path()).unwrap();
        assert!(src.next_event().is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd percolator-sim && cargo test --lib data::binance`
Expected: FAIL — `BinanceTradeSource` not defined

Note: add `tempfile = "3"` to `[dev-dependencies]` in Cargo.toml.

- [ ] **Step 3: Implement BinanceTradeSource**

In `src/data/binance.rs`:

```rust
use crate::{DataSource, MarketEvent, POS_SCALE};
use std::path::Path;

pub struct BinanceTradeSource {
    reader: csv::Reader<std::fs::File>,
}

impl BinanceTradeSource {
    pub fn from_path(path: &Path) -> Result<Self, csv::Error> {
        let reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_path(path)?;
        Ok(Self { reader })
    }
}

impl DataSource for BinanceTradeSource {
    fn next_event(&mut self) -> Option<MarketEvent> {
        let mut record = csv::StringRecord::new();
        if !self.reader.read_record(&mut record).ok()? {
            return None;
        }
        if record.len() < 7 {
            return None;
        }
        let price_str = record.get(1)?;
        let qty_str = record.get(2)?;
        let timestamp_str = record.get(5)?;
        let is_buyer_maker_str = record.get(6)?;

        let price_f: f64 = price_str.parse().ok()?;
        let qty_f: f64 = qty_str.parse().ok()?;
        let timestamp_ms: u64 = timestamp_str.parse().ok()?;
        let is_buyer_maker: bool = is_buyer_maker_str.parse().ok()?;

        let price = (price_f * POS_SCALE as f64) as u64;
        let qty = (qty_f * POS_SCALE as f64) as u128;
        let is_buy = !is_buyer_maker;

        Some(MarketEvent::Trade {
            timestamp_ms,
            price,
            qty,
            is_buy,
        })
    }
}
```

Update `src/data/mod.rs`:
```rust
pub mod binance;
pub mod tardis;
```

- [ ] **Step 4: Run tests**

Run: `cd percolator-sim && cargo test --lib data::binance`
Expected: 3 tests PASS

- [ ] **Step 5: Commit**

```bash
git add percolator-sim/src/data/ percolator-sim/Cargo.toml
git commit -m "feat(sim): implement Binance aggTrades CSV parser"
```

---

### Task 8: Tardis Order Book Parser

**Files:**
- Modify: `percolator-sim/src/data/tardis.rs`
- Test: inline `#[cfg(test)]` in `tardis.rs`

- [ ] **Step 1: Write failing tests**

In `src/data/tardis.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::MarketEvent;
    use std::io::Write;

    fn make_csv(rows: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        // Tardis normalized format has headers
        writeln!(f, "exchange,symbol,timestamp,local_timestamp,is_snapshot,side,price,amount").unwrap();
        write!(f, "{}", rows).unwrap();
        f
    }

    #[test]
    fn parse_book_snapshot() {
        let csv = concat!(
            "binance,BTCUSDT,2022-05-09T00:00:00.000Z,2022-05-09T00:00:00.100Z,true,bid,30000.0,1.5\n",
            "binance,BTCUSDT,2022-05-09T00:00:00.000Z,2022-05-09T00:00:00.100Z,true,bid,29999.0,2.0\n",
            "binance,BTCUSDT,2022-05-09T00:00:00.000Z,2022-05-09T00:00:00.100Z,true,ask,30001.0,1.0\n",
        );
        let f = make_csv(csv);
        let mut src = TardisBookSource::from_path(f.path()).unwrap();
        let event = src.next_event().unwrap();
        match event {
            MarketEvent::BookUpdate { bids, asks, .. } => {
                assert_eq!(bids.len(), 2);
                assert_eq!(asks.len(), 1);
            }
            _ => panic!("expected BookUpdate"),
        }
    }

    #[test]
    fn empty_after_all_consumed() {
        let csv = "binance,BTCUSDT,2022-05-09T00:00:00.000Z,2022-05-09T00:00:00.100Z,true,bid,30000.0,1.5\n";
        let f = make_csv(csv);
        let mut src = TardisBookSource::from_path(f.path()).unwrap();
        assert!(src.next_event().is_some());
        assert!(src.next_event().is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd percolator-sim && cargo test --lib data::tardis`
Expected: FAIL — `TardisBookSource` not defined

- [ ] **Step 3: Implement TardisBookSource**

```rust
use crate::{DataSource, MarketEvent, POS_SCALE};
use std::path::Path;

pub struct TardisBookSource {
    reader: csv::Reader<std::fs::File>,
    pending_bids: Vec<(u64, u128)>,
    pending_asks: Vec<(u64, u128)>,
    pending_timestamp_ms: u64,
    has_pending: bool,
}

impl TardisBookSource {
    pub fn from_path(path: &Path) -> Result<Self, csv::Error> {
        let reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(path)?;
        Ok(Self {
            reader,
            pending_bids: Vec::new(),
            pending_asks: Vec::new(),
            pending_timestamp_ms: 0,
            has_pending: false,
        })
    }

    fn parse_timestamp(ts: &str) -> Option<u64> {
        let dt = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
        Some(dt.timestamp_millis() as u64)
    }
}

impl DataSource for TardisBookSource {
    fn next_event(&mut self) -> Option<MarketEvent> {
        loop {
            let mut record = csv::StringRecord::new();
            let has_more = self.reader.read_record(&mut record).ok()?;

            if !has_more {
                if self.has_pending {
                    self.has_pending = false;
                    return Some(MarketEvent::BookUpdate {
                        timestamp_ms: self.pending_timestamp_ms,
                        bids: std::mem::take(&mut self.pending_bids),
                        asks: std::mem::take(&mut self.pending_asks),
                    });
                }
                return None;
            }

            if record.len() < 8 {
                continue;
            }

            let ts_str = record.get(2)?;
            let side = record.get(5)?;
            let price_str = record.get(6)?;
            let amount_str = record.get(7)?;

            let ts = Self::parse_timestamp(ts_str)?;
            let price_f: f64 = price_str.parse().ok()?;
            let amount_f: f64 = amount_str.parse().ok()?;
            let price = (price_f * POS_SCALE as f64) as u64;
            let amount = (amount_f * POS_SCALE as f64) as u128;

            if self.has_pending && ts != self.pending_timestamp_ms {
                let event = MarketEvent::BookUpdate {
                    timestamp_ms: self.pending_timestamp_ms,
                    bids: std::mem::take(&mut self.pending_bids),
                    asks: std::mem::take(&mut self.pending_asks),
                };
                self.pending_timestamp_ms = ts;
                match side {
                    "bid" => self.pending_bids.push((price, amount)),
                    "ask" => self.pending_asks.push((price, amount)),
                    _ => {}
                }
                return Some(event);
            }

            self.has_pending = true;
            self.pending_timestamp_ms = ts;
            match side {
                "bid" => self.pending_bids.push((price, amount)),
                "ask" => self.pending_asks.push((price, amount)),
                _ => {}
            }
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd percolator-sim && cargo test --lib data::tardis`
Expected: 2 tests PASS

- [ ] **Step 5: Commit**

```bash
git add percolator-sim/src/data/tardis.rs
git commit -m "feat(sim): implement Tardis order book CSV parser"
```

---

### Task 9: Account Manager

**Files:**
- Modify: `percolator-sim/src/engine/accounts.rs`
- Test: inline `#[cfg(test)]` in `accounts.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_manager_all_free() {
        let am = AccountManager::new(60, 4);
        assert_eq!(am.next_trade_account(), Some(0));
    }

    #[test]
    fn allocate_round_robin() {
        let mut am = AccountManager::new(60, 4);
        assert_eq!(am.allocate_trade_account(), Some(0));
        assert_eq!(am.allocate_trade_account(), Some(1));
        assert_eq!(am.allocate_trade_account(), Some(2));
    }

    #[test]
    fn release_makes_available() {
        let mut am = AccountManager::new(2, 2);
        assert_eq!(am.allocate_trade_account(), Some(0));
        assert_eq!(am.allocate_trade_account(), Some(1));
        assert_eq!(am.allocate_trade_account(), None); // full
        am.release_trade_account(0);
        assert_eq!(am.allocate_trade_account(), Some(0));
    }

    #[test]
    fn lp_accounts_separate() {
        let am = AccountManager::new(60, 4);
        let lps = am.lp_accounts();
        assert_eq!(lps, vec![60, 61, 62, 63]);
    }

    #[test]
    fn active_positions_tracked() {
        let mut am = AccountManager::new(60, 4);
        am.allocate_trade_account();
        am.mark_positioned(0);
        assert_eq!(am.positioned_accounts(), vec![0]);
        am.mark_flat(0);
        assert!(am.positioned_accounts().is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd percolator-sim && cargo test --lib engine::accounts`
Expected: FAIL

- [ ] **Step 3: Implement AccountManager**

```rust
use percolator::MAX_ACCOUNTS;

pub struct AccountManager {
    trade_count: u16,
    lp_count: u16,
    free_queue: Vec<u16>,
    positioned: Vec<bool>,
    next_robin: usize,
}

impl AccountManager {
    pub fn new(trade_accounts: u16, lp_accounts: u16) -> Self {
        assert!((trade_accounts + lp_accounts) as usize <= MAX_ACCOUNTS);
        let mut free_queue = Vec::with_capacity(trade_accounts as usize);
        for i in 0..trade_accounts {
            free_queue.push(i);
        }
        Self {
            trade_count: trade_accounts,
            lp_count: lp_accounts,
            free_queue,
            positioned: vec![false; MAX_ACCOUNTS],
            next_robin: 0,
        }
    }

    pub fn next_trade_account(&self) -> Option<u16> {
        self.free_queue.first().copied()
    }

    pub fn allocate_trade_account(&mut self) -> Option<u16> {
        if self.free_queue.is_empty() {
            return None;
        }
        let idx = self.next_robin % self.free_queue.len();
        let account = self.free_queue.remove(idx);
        self.next_robin = if self.free_queue.is_empty() { 0 } else { idx % self.free_queue.len() };
        Some(account)
    }

    pub fn release_trade_account(&mut self, idx: u16) {
        if idx < self.trade_count && !self.free_queue.contains(&idx) {
            self.free_queue.push(idx);
        }
    }

    pub fn lp_accounts(&self) -> Vec<u16> {
        (self.trade_count..self.trade_count + self.lp_count).collect()
    }

    pub fn mark_positioned(&mut self, idx: u16) {
        if (idx as usize) < self.positioned.len() {
            self.positioned[idx as usize] = true;
        }
    }

    pub fn mark_flat(&mut self, idx: u16) {
        if (idx as usize) < self.positioned.len() {
            self.positioned[idx as usize] = false;
        }
    }

    pub fn is_positioned(&self, idx: u16) -> bool {
        self.positioned.get(idx as usize).copied().unwrap_or(false)
    }

    pub fn positioned_accounts(&self) -> Vec<u16> {
        (0..self.trade_count)
            .filter(|&i| self.positioned[i as usize])
            .collect()
    }

    pub fn free_count(&self) -> usize {
        self.free_queue.len()
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd percolator-sim && cargo test --lib engine::accounts`
Expected: 5 tests PASS

- [ ] **Step 5: Commit**

```bash
git add percolator-sim/src/engine/accounts.rs
git commit -m "feat(sim): implement AccountManager with round-robin allocation"
```

---

### Task 10: Optimizer — Bounds + Rate Limiter

**Files:**
- Modify: `percolator-sim/src/optimizer/bounds.rs`
- Modify: `percolator-sim/src/optimizer/rate_limit.rs`
- Test: inline `#[cfg(test)]` in both files

- [ ] **Step 1: Write failing tests for bounds**

In `src/optimizer/bounds.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_within_bounds() {
        let b = ParamBounds::new(10.0, 100.0);
        assert_eq!(b.clamp(50.0), 50.0);
    }

    #[test]
    fn clamp_below_min() {
        let b = ParamBounds::new(10.0, 100.0);
        assert_eq!(b.clamp(5.0), 10.0);
    }

    #[test]
    fn clamp_above_max() {
        let b = ParamBounds::new(10.0, 100.0);
        assert_eq!(b.clamp(200.0), 100.0);
    }

    #[test]
    fn default_bounds_for_all_params() {
        let bounds = default_param_bounds();
        assert_eq!(bounds.len(), 8);
        for b in &bounds {
            assert!(b.min < b.max);
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd percolator-sim && cargo test --lib optimizer::bounds`
Expected: FAIL

- [ ] **Step 3: Implement bounds**

```rust
#[derive(Clone, Debug)]
pub struct ParamBounds {
    pub min: f64,
    pub max: f64,
}

impl ParamBounds {
    pub fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }

    pub fn clamp(&self, v: f64) -> f64 {
        v.max(self.min).min(self.max)
    }

    pub fn range(&self) -> f64 {
        self.max - self.min
    }
}

pub fn default_param_bounds() -> Vec<ParamBounds> {
    vec![
        ParamBounds::new(10.0, 1000.0),    // base_rate_per_slot
        ParamBounds::new(1.0, 3.0),        // leverage_exponent_num
        ParamBounds::new(1.0, 2.0),        // leverage_exponent_den
        ParamBounds::new(54000.0, 432000.0), // min_commitment_slots
        ParamBounds::new(2000.0, 8000.0),  // crowding_cap
        ParamBounds::new(1500.0, 5000.0),  // oi_vault_mult_max
        ParamBounds::new(2000.0, 10000.0), // pool_health_mult_max
        ParamBounds::new(1.0, 100.0),      // min_premium_per_slot
    ]
}
```

- [ ] **Step 4: Run bounds tests**

Run: `cd percolator-sim && cargo test --lib optimizer::bounds`
Expected: 4 tests PASS

- [ ] **Step 5: Write failing tests for rate limiter**

In `src/optimizer/rate_limit.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_limit_unchanged() {
        let rl = RateLimiter::new(0.10);
        assert_eq!(rl.limit(100.0, 105.0), 105.0);
    }

    #[test]
    fn exceeds_limit_clamped_up() {
        let rl = RateLimiter::new(0.10);
        assert_eq!(rl.limit(100.0, 120.0), 110.0);
    }

    #[test]
    fn exceeds_limit_clamped_down() {
        let rl = RateLimiter::new(0.10);
        assert_eq!(rl.limit(100.0, 80.0), 90.0);
    }

    #[test]
    fn zero_current_no_panic() {
        let rl = RateLimiter::new(0.10);
        assert_eq!(rl.limit(0.0, 50.0), 50.0);
    }
}
```

- [ ] **Step 6: Implement rate limiter**

```rust
pub struct RateLimiter {
    max_fraction: f64,
}

impl RateLimiter {
    pub fn new(max_fraction: f64) -> Self {
        Self { max_fraction }
    }

    pub fn limit(&self, current: f64, proposed: f64) -> f64 {
        if current == 0.0 {
            return proposed;
        }
        let max_delta = current.abs() * self.max_fraction;
        let delta = proposed - current;
        if delta.abs() <= max_delta {
            proposed
        } else {
            current + delta.signum() * max_delta
        }
    }
}

pub fn apply_rate_limits(current: &[f64], proposed: &[f64], max_fraction: f64) -> Vec<f64> {
    let rl = RateLimiter::new(max_fraction);
    current.iter().zip(proposed.iter())
        .map(|(&c, &p)| rl.limit(c, p))
        .collect()
}
```

- [ ] **Step 7: Run rate limiter tests**

Run: `cd percolator-sim && cargo test --lib optimizer::rate_limit`
Expected: 4 tests PASS

- [ ] **Step 8: Commit**

```bash
git add percolator-sim/src/optimizer/bounds.rs percolator-sim/src/optimizer/rate_limit.rs
git commit -m "feat(sim): implement parameter bounds and rate limiter for optimizer"
```

---

### Task 11: Optimizer — Nelder-Mead + Objective

**Files:**
- Modify: `percolator-sim/src/optimizer/mod.rs`
- Modify: `percolator-sim/src/optimizer/objective.rs`
- Test: inline `#[cfg(test)]` in both files

- [ ] **Step 1: Write failing tests for objective**

In `src/optimizer/objective.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feasible_positive_surplus() {
        let r = ObjectiveResult {
            fund_surplus: 1000,
            total_notional: 1_000_000,
            total_premiums: 500,
            budget_cap: 0.001,
        };
        let score = r.score();
        assert!(score > 0.0);
    }

    #[test]
    fn infeasible_returns_neg_infinity() {
        let r = ObjectiveResult {
            fund_surplus: 1000,
            total_notional: 1_000_000,
            total_premiums: 2000, // 0.2% > 0.1% cap
            budget_cap: 0.001,
        };
        assert_eq!(r.score(), f64::NEG_INFINITY);
    }

    #[test]
    fn zero_notional_returns_zero() {
        let r = ObjectiveResult {
            fund_surplus: 0,
            total_notional: 0,
            total_premiums: 0,
            budget_cap: 0.001,
        };
        assert_eq!(r.score(), 0.0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd percolator-sim && cargo test --lib optimizer::objective`
Expected: FAIL

- [ ] **Step 3: Implement objective**

```rust
pub struct ObjectiveResult {
    pub fund_surplus: u128,
    pub total_notional: u128,
    pub total_premiums: u128,
    pub budget_cap: f64,
}

impl ObjectiveResult {
    pub fn score(&self) -> f64 {
        if self.total_notional == 0 {
            return 0.0;
        }
        let premium_ratio = self.total_premiums as f64 / self.total_notional as f64;
        if premium_ratio > self.budget_cap {
            return f64::NEG_INFINITY;
        }
        self.fund_surplus as f64 / self.total_notional as f64
    }
}
```

- [ ] **Step 4: Run objective tests**

Run: `cd percolator-sim && cargo test --lib optimizer::objective`
Expected: 3 tests PASS

- [ ] **Step 5: Write failing tests for Nelder-Mead**

In `src/optimizer/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bounds::ParamBounds;

    #[test]
    fn optimize_simple_quadratic() {
        // Minimize (x - 3)^2 + (y - 7)^2 — optimum at (3, 7)
        let bounds = vec![
            ParamBounds::new(0.0, 10.0),
            ParamBounds::new(0.0, 10.0),
        ];
        let result = nelder_mead(
            &bounds,
            |params| -((params[0] - 3.0).powi(2) + (params[1] - 7.0).powi(2)),
            100,
            50,
            None,
        );
        assert!((result.best_params[0] - 3.0).abs() < 0.5);
        assert!((result.best_params[1] - 7.0).abs() < 0.5);
    }

    #[test]
    fn respects_bounds() {
        let bounds = vec![
            ParamBounds::new(5.0, 10.0), // optimum at 3 is outside
        ];
        let result = nelder_mead(
            &bounds,
            |params| -((params[0] - 3.0).powi(2)),
            50,
            20,
            None,
        );
        assert!(result.best_params[0] >= 5.0);
    }

    #[test]
    fn returns_after_max_iterations() {
        let bounds = vec![ParamBounds::new(0.0, 100.0)];
        let result = nelder_mead(
            &bounds,
            |params| -(params[0] - 50.0).powi(2),
            10,
            5,
            None,
        );
        assert!(result.iterations <= 10);
    }
}
```

- [ ] **Step 6: Implement Nelder-Mead**

```rust
pub mod bounds;
pub mod rate_limit;
pub mod objective;

use bounds::ParamBounds;

pub struct OptimizeResult {
    pub best_params: Vec<f64>,
    pub best_score: f64,
    pub iterations: u32,
}

pub fn nelder_mead<F>(
    bounds: &[ParamBounds],
    evaluate: F,
    max_iter: u32,
    stale_limit: u32,
    seed: Option<u64>,
) -> OptimizeResult
where
    F: Fn(&[f64]) -> f64,
{
    let n = bounds.len();
    let mut simplex: Vec<Vec<f64>> = Vec::with_capacity(n + 1);

    let mut rng_state = seed.unwrap_or(42);
    let mut next_rng = || -> f64 {
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((rng_state >> 33) as f64) / (u32::MAX as f64)
    };

    for i in 0..=n {
        let mut point = Vec::with_capacity(n);
        for j in 0..n {
            let v = if i == 0 {
                bounds[j].min + 0.5 * bounds[j].range()
            } else if i - 1 == j {
                bounds[j].min + 0.75 * bounds[j].range()
            } else {
                bounds[j].min + next_rng() * bounds[j].range()
            };
            point.push(bounds[j].clamp(v));
        }
        simplex.push(point);
    }

    let mut scores: Vec<f64> = simplex.iter().map(|p| evaluate(p)).collect();
    let mut best_score = f64::NEG_INFINITY;
    let mut best_params = simplex[0].clone();
    let mut stale_count = 0u32;

    let clamp_point = |p: &mut Vec<f64>| {
        for (i, v) in p.iter_mut().enumerate() {
            *v = bounds[i].clamp(*v);
        }
    };

    for iter in 0..max_iter {
        let mut order: Vec<usize> = (0..=n).collect();
        order.sort_by(|&a, &b| scores[b].partial_cmp(&scores[a]).unwrap());

        let best_idx = order[0];
        let worst_idx = order[n];
        let second_worst_idx = order[n - 1];

        if scores[best_idx] > best_score {
            best_score = scores[best_idx];
            best_params = simplex[best_idx].clone();
            stale_count = 0;
        } else {
            stale_count += 1;
        }

        if stale_count >= stale_limit {
            return OptimizeResult {
                best_params,
                best_score,
                iterations: iter,
            };
        }

        let diameter: f64 = (0..n).map(|d| {
            let vals: Vec<f64> = simplex.iter().map(|p| p[d]).collect();
            let mn = vals.iter().cloned().fold(f64::INFINITY, f64::min);
            let mx = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            (mx - mn) / bounds[d].range()
        }).fold(0.0f64, f64::max);

        if diameter < 0.01 {
            return OptimizeResult {
                best_params,
                best_score,
                iterations: iter,
            };
        }

        let mut centroid = vec![0.0; n];
        for &i in &order[..n] {
            for d in 0..n {
                centroid[d] += simplex[i][d];
            }
        }
        for d in 0..n {
            centroid[d] /= n as f64;
        }

        // Reflection
        let mut reflected: Vec<f64> = (0..n)
            .map(|d| 2.0 * centroid[d] - simplex[worst_idx][d])
            .collect();
        clamp_point(&mut reflected);
        let reflected_score = evaluate(&reflected);

        if reflected_score > scores[second_worst_idx] && reflected_score <= scores[best_idx] {
            simplex[worst_idx] = reflected;
            scores[worst_idx] = reflected_score;
            continue;
        }

        // Expansion
        if reflected_score > scores[best_idx] {
            let mut expanded: Vec<f64> = (0..n)
                .map(|d| 3.0 * centroid[d] - 2.0 * simplex[worst_idx][d])
                .collect();
            clamp_point(&mut expanded);
            let expanded_score = evaluate(&expanded);
            if expanded_score > reflected_score {
                simplex[worst_idx] = expanded;
                scores[worst_idx] = expanded_score;
            } else {
                simplex[worst_idx] = reflected;
                scores[worst_idx] = reflected_score;
            }
            continue;
        }

        // Contraction
        let mut contracted: Vec<f64> = (0..n)
            .map(|d| 0.5 * (centroid[d] + simplex[worst_idx][d]))
            .collect();
        clamp_point(&mut contracted);
        let contracted_score = evaluate(&contracted);

        if contracted_score > scores[worst_idx] {
            simplex[worst_idx] = contracted;
            scores[worst_idx] = contracted_score;
            continue;
        }

        // Shrink
        for i in 1..=n {
            let idx = order[i];
            for d in 0..n {
                simplex[idx][d] = 0.5 * (simplex[best_idx][d] + simplex[idx][d]);
                simplex[idx][d] = bounds[d].clamp(simplex[idx][d]);
            }
            scores[idx] = evaluate(&simplex[idx]);
        }
    }

    OptimizeResult {
        best_params,
        best_score,
        iterations: max_iter,
    }
}
```

- [ ] **Step 7: Run optimizer tests**

Run: `cd percolator-sim && cargo test --lib optimizer`
Expected: 10 tests PASS (4 bounds + 4 rate_limit + 3 objective + 3 nelder-mead)

- [ ] **Step 8: Commit**

```bash
git add percolator-sim/src/optimizer/
git commit -m "feat(sim): implement Nelder-Mead optimizer with bounds, rate limiter, and objective"
```

---

### Task 12: SimEngine Harness

**Files:**
- Modify: `percolator-sim/src/engine/mod.rs`
- Test: inline `#[cfg(test)]` in `mod.rs`

- [ ] **Step 1: Write failing tests**

In `src/engine/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MarketEvent, POS_SCALE};
    use percolator_insurance::PremiumParams;

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
    fn sim_engine_initializes() {
        let se = SimEngine::new(test_premium_params(), 400, 100);
        assert_eq!(se.clock.current_slot(), 0);
        assert_eq!(se.accounts.free_count(), 60);
    }

    #[test]
    fn process_trade_event_opens_position() {
        let mut se = SimEngine::new(test_premium_params(), 400, 100);
        let price: u64 = 50_000 * POS_SCALE as u64;
        se.initialize(price, 1_000_000_000);
        let event = MarketEvent::Trade {
            timestamp_ms: 400,
            price,
            qty: 1 * POS_SCALE as u128,
            is_buy: true,
        };
        let result = se.process_event(&event);
        assert!(result.is_ok());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd percolator-sim && cargo test --lib engine -- --test-threads=1`
Expected: FAIL — `SimEngine` not defined in `engine/mod.rs`

- [ ] **Step 3: Implement SimEngine**

```rust
pub mod accounts;
pub mod clock;

use accounts::AccountManager;
use clock::SlotClock;
use crate::metrics::{MetricsCollector, Snapshot};
use crate::signal::FlowSignal;
use crate::{MarketEvent, POS_SCALE};
use percolator::{
    LiquidationPolicy, RiskParams, MAX_ACCOUNTS,
};
use percolator_insurance::{
    InsuredRiskEngine, PremiumParams, InsuredError,
};

pub struct SimEngine {
    pub engine: InsuredRiskEngine,
    pub clock: SlotClock,
    pub accounts: AccountManager,
    pub metrics: MetricsCollector,
    pub signal: FlowSignal,
    last_oracle_price: u64,
    last_snapshot_slot: u64,
}

impl SimEngine {
    pub fn new(premium_params: PremiumParams, slot_duration_ms: u64, sample_interval: u64) -> Self {
        let risk_params = RiskParams {
            maintenance_margin_bps: 500,
            initial_margin_bps: 1000,
            trading_fee_bps: 10,
            max_accounts: 64,
            liquidation_fee_bps: 100,
            liquidation_fee_cap: percolator::U128::new(1_000_000_000),
            min_liquidation_abs: percolator::U128::new(0),
            min_nonzero_mm_req: 10,
            min_nonzero_im_req: 11,
            h_min: 0,
            h_max: 100,
            resolve_price_deviation_bps: 1000,
            max_accrual_dt_slots: 100,
            max_abs_funding_e9_per_slot: 10_000,
            min_funding_lifetime_slots: 10_000_000,
            max_active_positions_per_side: MAX_ACCOUNTS as u64,
            max_price_move_bps_per_slot: 3,
        };

        let engine = InsuredRiskEngine::new(risk_params, premium_params, 0, 1).unwrap();

        Self {
            engine,
            clock: SlotClock::new(0, slot_duration_ms),
            accounts: AccountManager::new(60, 4),
            metrics: MetricsCollector::new(sample_interval),
            signal: FlowSignal::new(),
            last_oracle_price: 1,
            last_snapshot_slot: 0,
        }
    }

    pub fn initialize(&mut self, oracle_price: u64, vault_seed: u128) {
        self.last_oracle_price = oracle_price;
        // Deposit into LP accounts to seed the vault
        let lps = self.accounts.lp_accounts();
        let per_lp = vault_seed / lps.len() as u128;
        for &lp in &lps {
            let _ = self.engine.deposit(lp, per_lp, 0);
        }
    }

    pub fn process_event(&mut self, event: &MarketEvent) -> Result<(), SimError> {
        match event {
            MarketEvent::Trade { timestamp_ms, price, qty, is_buy } => {
                let slots_advanced = self.clock.advance_to(*timestamp_ms);
                let now_slot = self.clock.current_slot();

                if *price > 0 {
                    self.last_oracle_price = *price;
                }

                self.signal.volume.record_trade(*timestamp_ms, (*qty).min(u64::MAX as u128) as u64, *is_buy);
                self.signal.aggression.record(*timestamp_ms, (*qty).min(u64::MAX as u128) as u64, *is_buy);

                let notional = (*qty as u128).saturating_mul(self.last_oracle_price as u128) / POS_SCALE;
                self.metrics.record_trade_notional(notional);

                if let Some(acct_idx) = self.accounts.allocate_trade_account() {
                    let leverage = 10u128;
                    let capital = notional / leverage;
                    if capital > 0 {
                        let _ = self.engine.deposit(acct_idx, capital.max(100), now_slot);

                        let lp = self.accounts.lp_accounts()[0];
                        let size_q = (*qty).min(i128::MAX as u128) as i128;
                        if size_q > 0 {
                            let (a, b) = if *is_buy { (acct_idx, lp) } else { (lp, acct_idx) };
                            match self.engine.execute_trade(
                                a, b, self.last_oracle_price, now_slot,
                                size_q, self.last_oracle_price, 0, 0, 100, None,
                            ) {
                                Ok(()) => {
                                    self.accounts.mark_positioned(acct_idx);
                                }
                                Err(_) => {
                                    self.accounts.release_trade_account(acct_idx);
                                }
                            }
                        } else {
                            self.accounts.release_trade_account(acct_idx);
                        }
                    } else {
                        self.accounts.release_trade_account(acct_idx);
                    }
                }

                if slots_advanced > 0 {
                    self.run_liquidation_sweep(now_slot);
                    self.maybe_snapshot(now_slot, *timestamp_ms);
                }
            }
            MarketEvent::BookUpdate { timestamp_ms, bids, asks } => {
                self.clock.advance_to(*timestamp_ms);
                let bid_levels: Vec<(u64, u64)> = bids.iter()
                    .map(|&(p, q)| (p, q.min(u64::MAX as u128) as u64))
                    .collect();
                let ask_levels: Vec<(u64, u64)> = asks.iter()
                    .map(|&(p, q)| (p, q.min(u64::MAX as u128) as u64))
                    .collect();
                self.signal.depth.update(&bid_levels, &ask_levels);
            }
        }
        Ok(())
    }

    fn run_liquidation_sweep(&mut self, now_slot: u64) {
        let positioned = self.accounts.positioned_accounts();
        for idx in positioned {
            match self.engine.liquidate(
                idx, now_slot, self.last_oracle_price,
                LiquidationPolicy::FullClose, 0, 0, 100, None,
            ) {
                Ok(true) => {
                    let capital = self.engine.engine.accounts[idx as usize].capital.get();
                    self.metrics.record_liquidation(now_slot, capital);
                    self.accounts.mark_flat(idx);
                    self.accounts.release_trade_account(idx);
                }
                _ => {}
            }
        }
    }

    fn maybe_snapshot(&mut self, slot: u64, timestamp_ms: u64) {
        if slot < self.last_snapshot_slot + self.metrics.sample_interval {
            return;
        }
        self.last_snapshot_slot = slot;

        let (h_num, h_den) = self.engine.engine.haircut_ratio();
        let active = self.accounts.positioned_accounts().len() as u32;
        let tox = self.signal.toxicity(timestamp_ms);

        self.metrics.record(Snapshot {
            slot,
            timestamp_ms,
            insurance_fund_balance: self.engine.engine.insurance_fund.balance.get(),
            pool_balance: self.engine.pool.balance,
            pool_total_collected: self.engine.pool.total_collected,
            pool_total_paid_out: self.engine.pool.total_paid_out,
            haircut_num: h_num,
            haircut_den: h_den,
            vault_balance: self.engine.engine.vault.get(),
            total_oi_long: self.engine.engine.oi_eff_long_q,
            total_oi_short: self.engine.engine.oi_eff_short_q,
            active_accounts: active,
            flow_toxicity: tox,
        });
    }

    pub fn fund_balance(&self) -> u128 {
        self.engine.engine.insurance_fund.balance.get()
    }

    pub fn conservation_ok(&self) -> bool {
        self.engine.engine.check_conservation()
    }
}

#[derive(Debug)]
pub enum SimError {
    Engine(InsuredError),
    Data(String),
}

impl From<InsuredError> for SimError {
    fn from(e: InsuredError) -> Self {
        SimError::Engine(e)
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd percolator-sim && cargo test --lib engine -- --test-threads=1`
Expected: tests PASS (clock tests + engine tests)

- [ ] **Step 5: Commit**

```bash
git add percolator-sim/src/engine/
git commit -m "feat(sim): implement SimEngine harness with liquidation sweep and metrics snapshots"
```

---

### Task 13: sim-replay Binary

**Files:**
- Modify: `percolator-sim/bin/sim_replay.rs`

- [ ] **Step 1: Implement sim-replay**

```rust
use clap::Parser;
use percolator_insurance::PremiumParams;
use percolator_sim::data::binance::BinanceTradeSource;
use percolator_sim::engine::SimEngine;
use percolator_sim::metrics::report::{generate_report, write_report, ReportConfig};
use percolator_sim::{DataSource, POS_SCALE};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "sim-replay", about = "Replay historical data through InsuredRiskEngine")]
struct Args {
    #[arg(long)]
    data: PathBuf,
    #[arg(long, default_value = "binance-trades")]
    format: String,
    #[arg(long)]
    params: Option<PathBuf>,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long, default_value_t = u64::MAX)]
    slots: u64,
    #[arg(long, default_value_t = 60)]
    accounts: u16,
    #[arg(long, default_value_t = 0.1)]
    budget_cap: f64,
}

fn default_premium_params() -> PremiumParams {
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

fn main() {
    let args = Args::parse();

    let params = if let Some(params_path) = &args.params {
        let json = std::fs::read_to_string(params_path)
            .expect("failed to read params file");
        serde_json::from_str(&json).expect("failed to parse params JSON")
    } else {
        default_premium_params()
    };

    let init_price: u64 = 50_000 * POS_SCALE as u64;
    let vault_seed: u128 = 10_000_000_000;

    let mut engine = SimEngine::new(params.clone(), 400, 100);
    engine.initialize(init_price, vault_seed);

    let fund_start = engine.fund_balance();

    let mut source: Box<dyn DataSource> = match args.format.as_str() {
        "binance-trades" => {
            Box::new(BinanceTradeSource::from_path(&args.data).expect("failed to open data file"))
        }
        _ => {
            eprintln!("unsupported format: {}", args.format);
            std::process::exit(1);
        }
    };

    let mut event_count = 0u64;
    while let Some(event) = source.next_event() {
        if engine.clock.current_slot() >= args.slots {
            break;
        }
        let _ = engine.process_event(&event);
        event_count += 1;
        if event_count % 100_000 == 0 {
            eprint!("\r  processed {} events, slot {}", event_count, engine.clock.current_slot());
        }
    }
    eprintln!("\n  done: {} events, {} slots", event_count, engine.clock.current_slot());

    let fund_end = engine.fund_balance();
    let total_slots = engine.clock.current_slot();

    let config = ReportConfig {
        scenario_name: args.data.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "replay".to_string()),
        params,
        budget_cap_pct: args.budget_cap,
        fund_start,
        fund_end,
        total_slots,
        slot_duration_ms: 400,
    };

    let report = generate_report(&engine.metrics, &config);
    print!("{}", report);

    let output_path = args.output.unwrap_or_else(|| {
        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        PathBuf::from(format!("output/replay-{}.txt", timestamp))
    });
    write_report(&report, &output_path).expect("failed to write report");
    eprintln!("  report saved to {}", output_path.display());
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd percolator-sim && cargo build --bin sim-replay`
Expected: compiles successfully

- [ ] **Step 3: Run with --help**

Run: `cd percolator-sim && cargo run --bin sim-replay -- --help`
Expected: shows usage with --data, --format, --output flags

- [ ] **Step 4: Commit**

```bash
git add percolator-sim/bin/sim_replay.rs
git commit -m "feat(sim): implement sim-replay binary with Binance CSV support"
```

---

### Task 14: sim-optimize Binary

**Files:**
- Modify: `percolator-sim/bin/sim_optimize.rs`

- [ ] **Step 1: Implement sim-optimize**

```rust
use clap::Parser;
use percolator_insurance::PremiumParams;
use percolator_sim::data::binance::BinanceTradeSource;
use percolator_sim::engine::SimEngine;
use percolator_sim::metrics::report::{generate_report, write_report, ReportConfig};
use percolator_sim::optimizer::bounds::default_param_bounds;
use percolator_sim::optimizer::rate_limit::apply_rate_limits;
use percolator_sim::optimizer::nelder_mead;
use percolator_sim::{DataSource, POS_SCALE};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "sim-optimize", about = "Search for optimal PremiumParams")]
struct Args {
    #[arg(long)]
    data: PathBuf,
    #[arg(long, default_value = "binance-trades")]
    format: String,
    #[arg(long, default_value_t = 0.1)]
    budget_cap: f64,
    #[arg(long, default_value_t = 500)]
    max_iter: u32,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long)]
    seed: Option<u64>,
}

fn params_from_vec(v: &[f64]) -> PremiumParams {
    PremiumParams {
        base_rate_per_slot: v[0] as u128,
        leverage_exponent_num: v[1] as u64,
        leverage_exponent_den: v[2] as u64,
        min_commitment_slots: v[3] as u64,
        crowding_low_ratio_num: 1500,
        crowding_low_ratio_den: 1000,
        crowding_high_ratio_num: 5000,
        crowding_high_ratio_den: 1000,
        crowding_cap: v[4] as u128,
        oi_vault_floor_ratio_num: 1,
        oi_vault_floor_ratio_den: 1,
        oi_vault_cap_ratio_num: 5,
        oi_vault_cap_ratio_den: 1,
        oi_vault_mult_max: v[5] as u128,
        pool_health_low_num: 1,
        pool_health_low_den: 100,
        pool_health_high_num: 5,
        pool_health_high_den: 100,
        pool_health_mult_max: v[6] as u128,
        min_premium_per_slot: v[7] as u128,
    }
}

fn run_sim(data_path: &PathBuf, params: PremiumParams, budget_cap: f64) -> f64 {
    let init_price: u64 = 50_000 * POS_SCALE as u64;
    let vault_seed: u128 = 10_000_000_000;

    let mut engine = SimEngine::new(params, 400, 100);
    engine.initialize(init_price, vault_seed);
    let fund_start = engine.fund_balance();

    let mut source = match BinanceTradeSource::from_path(data_path) {
        Ok(s) => s,
        Err(_) => return f64::NEG_INFINITY,
    };

    while let Some(event) = source.next_event() {
        let _ = engine.process_event(&event);
    }

    let fund_end = engine.fund_balance();
    let total_notional = engine.metrics.total_notional_traded;
    let pool_collected = engine.metrics.snapshots.last()
        .map(|s| s.pool_total_collected).unwrap_or(0);

    if total_notional == 0 {
        return 0.0;
    }

    let premium_ratio = pool_collected as f64 / total_notional as f64;
    if premium_ratio > budget_cap / 100.0 {
        return f64::NEG_INFINITY;
    }

    let surplus = if fund_end >= fund_start { fund_end - fund_start } else { 0 };
    surplus as f64 / total_notional as f64
}

fn main() {
    let args = Args::parse();
    let bounds = default_param_bounds();

    eprintln!("  starting optimizer: {} max iterations", args.max_iter);

    let data_path = args.data.clone();
    let budget = args.budget_cap;

    let result = nelder_mead(
        &bounds,
        |p| run_sim(&data_path, params_from_vec(p), budget),
        args.max_iter,
        50,
        args.seed,
    );

    let best_params = params_from_vec(&result.best_params);
    eprintln!("  optimizer done: {} iterations, best score = {:.8}", result.iterations, result.best_score);
    eprintln!("  best params: {:?}", result.best_params);

    // Run final sim with best params and generate report
    let init_price: u64 = 50_000 * POS_SCALE as u64;
    let mut engine = SimEngine::new(best_params.clone(), 400, 100);
    engine.initialize(init_price, 10_000_000_000);
    let fund_start = engine.fund_balance();

    if let Ok(mut source) = BinanceTradeSource::from_path(&args.data) {
        while let Some(event) = source.next_event() {
            let _ = engine.process_event(&event);
        }
    }

    let config = ReportConfig {
        scenario_name: format!("optimize-best-{}", result.iterations),
        params: best_params,
        budget_cap_pct: args.budget_cap,
        fund_start,
        fund_end: engine.fund_balance(),
        total_slots: engine.clock.current_slot(),
        slot_duration_ms: 400,
    };

    let report = generate_report(&engine.metrics, &config);
    print!("{}", report);

    let output_path = args.output.unwrap_or_else(|| {
        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        PathBuf::from(format!("output/optimize-{}.txt", timestamp))
    });
    write_report(&report, &output_path).expect("failed to write report");
    eprintln!("  report saved to {}", output_path.display());
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd percolator-sim && cargo build --bin sim-optimize`
Expected: compiles successfully

- [ ] **Step 3: Commit**

```bash
git add percolator-sim/bin/sim_optimize.rs
git commit -m "feat(sim): implement sim-optimize binary with Nelder-Mead parameter search"
```

---

### Task 15: sim-live Binary (Prototype)

**Files:**
- Modify: `percolator-sim/src/feed/binance_ws.rs`
- Modify: `percolator-sim/src/feed/mod.rs`
- Modify: `percolator-sim/bin/sim_live.rs`

- [ ] **Step 1: Implement Binance websocket feed**

In `src/feed/binance_ws.rs`:

```rust
use crate::{MarketEvent, POS_SCALE};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use futures_util::StreamExt;
use tokio::sync::mpsc;

pub async fn connect_binance_trades(
    symbol: &str,
    tx: mpsc::Sender<MarketEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("wss://fstream.binance.com/ws/{}@aggTrade", symbol.to_lowercase());
    let (ws_stream, _) = connect_async(&url).await?;
    let (_, mut read) = ws_stream.split();

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Ping(_)) => continue,
            Ok(_) => continue,
            Err(_) => break,
        };

        let v: serde_json::Value = match serde_json::from_str(&msg) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let price_f: f64 = v["p"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let qty_f: f64 = v["q"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let timestamp_ms = v["T"].as_u64().unwrap_or(0);
        let is_buyer_maker = v["m"].as_bool().unwrap_or(false);

        let event = MarketEvent::Trade {
            timestamp_ms,
            price: (price_f * POS_SCALE as f64) as u64,
            qty: (qty_f * POS_SCALE as f64) as u128,
            is_buy: !is_buyer_maker,
        };

        if tx.send(event).await.is_err() {
            break;
        }
    }

    Ok(())
}
```

Update `src/feed/mod.rs`:
```rust
pub mod binance_ws;
pub mod bybit_ws;
pub mod okx_ws;
pub mod coinbase_ws;
```

Leave `bybit_ws.rs`, `okx_ws.rs`, `coinbase_ws.rs` as empty stubs for now.

Add `futures-util = "0.3"` to `[dependencies]` in Cargo.toml.

- [ ] **Step 2: Implement sim-live binary**

```rust
use clap::Parser;
use percolator_insurance::PremiumParams;
use percolator_sim::engine::SimEngine;
use percolator_sim::feed::binance_ws::connect_binance_trades;
use percolator_sim::metrics::report::{generate_report, write_report, ReportConfig};
use percolator_sim::POS_SCALE;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

#[derive(Parser)]
#[command(name = "sim-live", about = "Live exchange feed simulation")]
struct Args {
    #[arg(long, default_value = "binance")]
    exchanges: String,
    #[arg(long, default_value = "BTCUSDT")]
    symbol: String,
    #[arg(long)]
    params: Option<PathBuf>,
    #[arg(long, default_value_t = 3600)]
    duration: u64,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long, default_value_t = 0.1)]
    budget_cap: f64,
}

fn default_premium_params() -> PremiumParams {
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

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let params = if let Some(params_path) = &args.params {
        let json = std::fs::read_to_string(params_path).expect("failed to read params");
        serde_json::from_str(&json).expect("failed to parse params")
    } else {
        default_premium_params()
    };

    let init_price: u64 = 50_000 * POS_SCALE as u64;
    let mut engine = SimEngine::new(params.clone(), 400, 100);
    engine.initialize(init_price, 10_000_000_000);
    let fund_start = engine.fund_balance();

    let (tx, mut rx) = mpsc::channel(10_000);

    let symbol = args.symbol.clone();
    tokio::spawn(async move {
        if let Err(e) = connect_binance_trades(&symbol, tx).await {
            eprintln!("  websocket error: {}", e);
        }
    });

    eprintln!("  connected to Binance, running for {}s...", args.duration);

    let deadline = Duration::from_secs(args.duration);
    let mut event_count = 0u64;

    let _ = timeout(deadline, async {
        while let Some(event) = rx.recv().await {
            let _ = engine.process_event(&event);
            event_count += 1;
            if event_count % 1000 == 0 {
                eprint!("\r  {} events, slot {}, toxicity: {}", event_count, engine.clock.current_slot(), engine.signal.toxicity(0));
            }
        }
    }).await;

    eprintln!("\n  done: {} events", event_count);

    let config = ReportConfig {
        scenario_name: format!("live-{}", args.symbol),
        params,
        budget_cap_pct: args.budget_cap,
        fund_start,
        fund_end: engine.fund_balance(),
        total_slots: engine.clock.current_slot(),
        slot_duration_ms: 400,
    };

    let report = generate_report(&engine.metrics, &config);
    print!("{}", report);

    let output_path = args.output.unwrap_or_else(|| {
        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        PathBuf::from(format!("output/live-{}.txt", timestamp))
    });
    write_report(&report, &output_path).expect("failed to write report");
    eprintln!("  report saved to {}", output_path.display());
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cd percolator-sim && cargo build --bin sim-live`
Expected: compiles successfully

- [ ] **Step 4: Commit**

```bash
git add percolator-sim/
git commit -m "feat(sim): implement sim-live binary with Binance websocket feed"
```

---

### Task 16: Integration Test + Final Verification

**Files:**
- Create: `percolator-sim/tests/integration_test.rs`

- [ ] **Step 1: Write integration test**

```rust
use percolator_insurance::PremiumParams;
use percolator_sim::engine::SimEngine;
use percolator_sim::{MarketEvent, POS_SCALE};

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
fn full_replay_synthetic_data() {
    let price: u64 = 50_000 * POS_SCALE as u64;
    let mut engine = SimEngine::new(test_premium_params(), 400, 100);
    engine.initialize(price, 10_000_000_000);

    let events: Vec<MarketEvent> = (0..100).map(|i| {
        MarketEvent::Trade {
            timestamp_ms: i * 500,
            price,
            qty: POS_SCALE as u128,
            is_buy: i % 2 == 0,
        }
    }).collect();

    for event in &events {
        let result = engine.process_event(event);
        assert!(result.is_ok());
    }

    assert!(engine.conservation_ok());
    assert!(!engine.metrics.snapshots.is_empty());
}

#[test]
fn report_generation() {
    use percolator_sim::metrics::report::{generate_report, ReportConfig};

    let price: u64 = 50_000 * POS_SCALE as u64;
    let mut engine = SimEngine::new(test_premium_params(), 400, 100);
    engine.initialize(price, 10_000_000_000);
    let fund_start = engine.fund_balance();

    for i in 0..50 {
        let event = MarketEvent::Trade {
            timestamp_ms: i * 500,
            price,
            qty: POS_SCALE as u128,
            is_buy: true,
        };
        let _ = engine.process_event(&event);
    }

    let config = ReportConfig {
        scenario_name: "test".to_string(),
        params: test_premium_params(),
        budget_cap_pct: 0.1,
        fund_start,
        fund_end: engine.fund_balance(),
        total_slots: engine.clock.current_slot(),
        slot_duration_ms: 400,
    };

    let report = generate_report(&engine.metrics, &config);
    assert!(report.contains("PERCOLATOR-SIM REPORT"));
    assert!(report.contains("PARAMETERS"));
    assert!(report.contains("FUND HEALTH"));
    assert!(report.contains("PREMIUMS"));
    assert!(report.contains("LIQUIDATIONS"));
    assert!(report.contains("FLOW SIGNAL"));
    assert!(report.contains("VERDICT"));
}
```

- [ ] **Step 2: Run integration tests**

Run: `cd percolator-sim && cargo test --test integration_test -- --test-threads=1`
Expected: 2 tests PASS

- [ ] **Step 3: Run all tests**

Run: `cd percolator-sim && cargo test -- --test-threads=1`
Expected: all tests PASS

- [ ] **Step 4: Run Percolator tests to verify no regression**

Run: `cd ~/percolator && cargo test -- --test-threads=1`
Expected: 283 Percolator tests + 66 insurance tests PASS

- [ ] **Step 5: Commit**

```bash
git add percolator-sim/tests/
git commit -m "feat(sim): add integration tests for SimEngine and report generation"
```

---

### Task 17: Add serde support to PremiumParams

The sim-replay and sim-optimize binaries need to load PremiumParams from JSON.

**Files:**
- Modify: `percolator-insurance/src/wrapper.rs`
- Modify: `percolator-insurance/Cargo.toml`

- [ ] **Step 1: Add serde dependency**

Add to `percolator-insurance/Cargo.toml`:
```toml
[dependencies]
percolator = { path = "../", features = ["test"] }
serde = { version = "1", features = ["derive"], optional = true }

[features]
default = []
test = []
serde = ["dep:serde"]
```

- [ ] **Step 2: Add serde derives to PremiumParams**

In `wrapper.rs`, change:
```rust
#[derive(Clone, Copy, Debug)]
pub struct PremiumParams {
```
to:
```rust
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PremiumParams {
```

- [ ] **Step 3: Update percolator-sim/Cargo.toml dependency**

Change to:
```toml
percolator-insurance = { path = "../percolator-insurance", features = ["serde"] }
```

- [ ] **Step 4: Verify both crates compile**

Run: `cd ~/percolator && cargo test -p percolator-insurance -- --test-threads=1 && cargo check -p percolator-sim`
Expected: all pass/compile

- [ ] **Step 5: Commit**

```bash
git add percolator-insurance/Cargo.toml percolator-insurance/src/wrapper.rs percolator-sim/Cargo.toml
git commit -m "feat(insurance): add optional serde support for PremiumParams"
```
