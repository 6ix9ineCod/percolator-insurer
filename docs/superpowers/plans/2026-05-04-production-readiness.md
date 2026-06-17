# Percolator Production Readiness — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the insurance premium optimizer robust across market regimes and validate the live feed pipeline.

**Architecture:** Add multi-day min-scoring to the Nelder-Mead optimizer so params must survive every day in a 7-day training set. Introduce a SimConfig JSON format for portable params. Fix the live binary to use real market prices.

**Tech Stack:** Rust, clap, serde_json, tokio-tungstenite, Binance data.binance.vision archives + futures websocket API

---

### Task 1: Download and Validate Market Data

**Files:**
- Create: `data/download.sh`
- Create: `data/` directory with 7 CSV files

- [ ] **Step 1: Create the data directory**

```bash
mkdir -p /home/acheron28nyx/percolator/data
```

- [ ] **Step 2: Write the download script**

Create `data/download.sh`:

```bash
#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

BASE_URL="https://data.binance.vision/data/futures/um/daily/aggTrades/BTCUSDT"

for DATE in 2026-04-21 2026-04-22 2026-04-23 2026-04-24 2026-04-25 2026-04-26 2026-04-27; do
    FILE="BTCUSDT-aggTrades-${DATE}.csv"
    ZIP="BTCUSDT-aggTrades-${DATE}.zip"

    if [ -f "$FILE" ] && [ -s "$FILE" ]; then
        echo "SKIP: $FILE already exists"
        continue
    fi

    echo "Downloading $ZIP..."
    curl -fSL "${BASE_URL}/${ZIP}" -o "$ZIP"
    unzip -o "$ZIP"
    rm -f "$ZIP"

    # Validate: non-empty and parseable
    if [ ! -s "$FILE" ]; then
        echo "ERROR: $FILE is empty after extraction"
        exit 1
    fi

    LINE_COUNT=$(wc -l < "$FILE")
    if [ "$LINE_COUNT" -lt 10 ]; then
        echo "ERROR: $FILE has only $LINE_COUNT lines"
        exit 1
    fi

    # Validate CSV structure: column 1 = price (float), column 5 = timestamp (int)
    SAMPLE=$(head -1 "$FILE" | awk -F, '{print NF}')
    if [ "$SAMPLE" -lt 7 ]; then
        echo "ERROR: $FILE has $SAMPLE columns, expected ≥7"
        exit 1
    fi

    echo "OK: $FILE ($LINE_COUNT lines)"
done

echo ""
echo "All files validated:"
ls -lh BTCUSDT-aggTrades-2026-04-2*.csv
```

- [ ] **Step 3: Run the download script**

```bash
chmod +x data/download.sh
cd /home/acheron28nyx/percolator && bash data/download.sh
```

Expected: 7 CSV files in `data/`, each with millions of lines. Apr 25-27 may need downloading too since they weren't found on disk.

- [ ] **Step 4: Verify all 7 files exist**

```bash
ls -lh data/BTCUSDT-aggTrades-2026-04-2*.csv | wc -l
```

Expected: `7`

- [ ] **Step 5: Commit**

```bash
cd /home/acheron28nyx/percolator
echo "data/*.csv" >> .gitignore
git add data/download.sh .gitignore
git commit -m "feat(sim): add Binance aggTrades download script for 7-day training data"
```

---

### Task 2: Update Parameter Bounds

**Files:**
- Modify: `percolator-sim/src/optimizer/bounds.rs:21-31`
- Test: existing tests in same file

- [ ] **Step 1: Update the bounds**

In `percolator-sim/src/optimizer/bounds.rs`, change `default_param_bounds()`:

```rust
pub fn default_param_bounds() -> Vec<ParamBounds> {
    vec![
        ParamBounds::new(10.0, 1000.0),       // [0] base_rate_per_slot
        ParamBounds::new(1.3, 3.0),           // [1] leverage_exponent (floor 1.3 → effective 1.5 with denom 4)
        ParamBounds::new(100.0, 2700.0),      // [2] min_commitment_slots (40ms – 18min at 400ms slots)
        ParamBounds::new(2000.0, 8000.0),     // [3] crowding_cap
        ParamBounds::new(1500.0, 5000.0),     // [4] oi_vault_mult_max
        ParamBounds::new(2000.0, 10000.0),    // [5] pool_health_mult_max
        ParamBounds::new(1.0, 100.0),         // [6] min_premium_per_slot
    ]
}
```

- [ ] **Step 2: Run existing tests**

```bash
cd /home/acheron28nyx/percolator && cargo test -p percolator-sim optimizer::bounds -- --nocapture
```

Expected: all 4 bounds tests pass.

- [ ] **Step 3: Commit**

```bash
git add percolator-sim/src/optimizer/bounds.rs
git commit -m "feat(sim): constrain leverage_exponent ≥1.3, min_commitment_slots ≤2700"
```

---

### Task 3: Add SimConfig Struct

**Files:**
- Create: `percolator-sim/src/config.rs`
- Modify: `percolator-sim/src/lib.rs:1` (add module)

- [ ] **Step 1: Write tests for SimConfig serialization**

Create `percolator-sim/src/config.rs`:

```rust
use percolator_insurance::PremiumParams;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SimConfig {
    pub premium_params: PremiumParams,
    pub fund_seed: u128,
    pub budget_cap: f64,
}

impl SimConfig {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let json = std::fs::read_to_string(path)?;
        let config: SimConfig = serde_json::from_str(&json)?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SimConfig {
        SimConfig {
            premium_params: PremiumParams {
                base_rate_per_slot: 213,
                leverage_exponent_num: 3,
                leverage_exponent_den: 2,
                min_commitment_slots: 2700,
                crowding_low_ratio_num: 1500,
                crowding_low_ratio_den: 1000,
                crowding_high_ratio_num: 5000,
                crowding_high_ratio_den: 1000,
                crowding_cap: 4186,
                oi_vault_floor_ratio_num: 1,
                oi_vault_floor_ratio_den: 1,
                oi_vault_cap_ratio_num: 5,
                oi_vault_cap_ratio_den: 1,
                oi_vault_mult_max: 2803,
                pool_health_low_num: 1,
                pool_health_low_den: 100,
                pool_health_high_num: 5,
                pool_health_high_den: 100,
                pool_health_mult_max: 2477,
                min_premium_per_slot: 13,
            },
            fund_seed: 50_000_000_000,
            budget_cap: 0.1,
        }
    }

    #[test]
    fn roundtrip_json() {
        let config = test_config();
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: SimConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.premium_params.base_rate_per_slot, 213);
        assert_eq!(parsed.fund_seed, 50_000_000_000);
        assert!((parsed.budget_cap - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn save_and_load() {
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sim-config.json");
        config.save(&path).unwrap();
        let loaded = SimConfig::load(&path).unwrap();
        assert_eq!(loaded.premium_params.leverage_exponent_num, 3);
        assert_eq!(loaded.premium_params.leverage_exponent_den, 2);
        assert_eq!(loaded.fund_seed, 50_000_000_000);
    }

    #[test]
    fn load_nonexistent_file_errors() {
        let result = SimConfig::load(Path::new("/tmp/nonexistent-sim-config.json"));
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Register the module**

In `percolator-sim/src/lib.rs`, add after the existing module declarations:

```rust
pub mod config;
```

- [ ] **Step 3: Run the tests**

```bash
cd /home/acheron28nyx/percolator && cargo test -p percolator-sim config:: -- --nocapture
```

Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add percolator-sim/src/config.rs percolator-sim/src/lib.rs
git commit -m "feat(sim): add SimConfig struct with JSON load/save"
```

---

### Task 4: Add --config Support to sim-replay

**Files:**
- Modify: `percolator-sim/bin/sim_replay.rs`

- [ ] **Step 1: Update imports and CLI args**

Replace the `Args` struct and `default_premium_params` function in `sim_replay.rs` with:

```rust
use clap::Parser;
use percolator_insurance::PremiumParams;
use percolator_sim::config::SimConfig;
use percolator_sim::data::binance::BinanceTradeSource;
use percolator_sim::engine::SimEngine;
use percolator_sim::metrics::report::{generate_report, write_report, ReportConfig};
use percolator_sim::{DataSource, POS_SCALE};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "sim-replay", about = "Replay historical data through InsuredRiskEngine")]
struct Args {
    #[arg(long)]
    data: PathBuf,
    #[arg(long, default_value = "binance-trades")]
    format: String,
    #[arg(long, group = "param_source")]
    params: Option<PathBuf>,
    #[arg(long, group = "param_source")]
    config: Option<PathBuf>,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long, default_value_t = u64::MAX)]
    slots: u64,
    #[arg(long, default_value_t = 60)]
    accounts: u16,
    #[arg(long, default_value_t = 0.1)]
    budget_cap: f64,
    #[arg(long, default_value_t = 0)]
    fund_seed: u128,
}
```

- [ ] **Step 2: Update main() to load from config**

Replace the param-loading block at the top of `main()` with:

```rust
fn main() {
    let args = Args::parse();
    let matches = <Args as clap::CommandFactory>::command().get_matches();

    let (params, fund_seed, budget_cap) = if let Some(config_path) = &args.config {
        let cfg = SimConfig::load(config_path).expect("failed to load config");
        let fs = if matches.value_source("fund_seed") == Some(clap::parser::ValueSource::CommandLine) {
            args.fund_seed
        } else {
            cfg.fund_seed
        };
        let bc = if matches.value_source("budget_cap") == Some(clap::parser::ValueSource::CommandLine) {
            args.budget_cap
        } else {
            cfg.budget_cap
        };
        (cfg.premium_params, fs, bc)
    } else if let Some(params_path) = &args.params {
        let json = std::fs::read_to_string(params_path).expect("failed to read params file");
        let p: PremiumParams = serde_json::from_str(&json).expect("failed to parse params JSON");
        (p, args.fund_seed, args.budget_cap)
    } else {
        (default_premium_params(), args.fund_seed, args.budget_cap)
    };

    let init_price: u64 = 50_000 * POS_SCALE as u64;
    let vault_seed: u128 = 10_000_000_000;

    let mut engine = SimEngine::new(params, 400, 100);
    engine.initialize(init_price, vault_seed, fund_seed);

    let fund_start = engine.fund_balance();

    // ... rest of main() stays the same, but replace args.fund_seed with fund_seed
    // and args.budget_cap with budget_cap in ReportConfig
```

Keep the existing `default_premium_params()` function and the rest of `main()` unchanged, except use the local `fund_seed` and `budget_cap` variables instead of `args.fund_seed` and `args.budget_cap`.

- [ ] **Step 3: Build and verify**

```bash
cd /home/acheron28nyx/percolator && cargo build --bin sim-replay
```

Expected: compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add percolator-sim/bin/sim_replay.rs
git commit -m "feat(sim): add --config flag to sim-replay for SimConfig loading"
```

---

### Task 5: Add Multi-Day Min-Scoring to Optimizer

**Files:**
- Modify: `percolator-sim/bin/sim_optimize.rs`

- [ ] **Step 1: Add --data-dir flag and multi-file loading**

Update the `Args` struct to support both `--data` and `--data-dir`:

```rust
use clap::Parser;
use percolator_insurance::PremiumParams;
use percolator_sim::config::SimConfig;
use percolator_sim::data::binance::BinanceTradeSource;
use percolator_sim::engine::SimEngine;
use percolator_sim::metrics::report::{generate_report, write_report, ReportConfig};
use percolator_sim::optimizer::bounds::default_param_bounds;
use percolator_sim::optimizer::nelder_mead;
use percolator_sim::{DataSource, POS_SCALE};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "sim-optimize", about = "Search for optimal PremiumParams")]
struct Args {
    #[arg(long, group = "data_source")]
    data: Option<PathBuf>,
    #[arg(long, group = "data_source")]
    data_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 0.1)]
    budget_cap: f64,
    #[arg(long, default_value_t = 500)]
    max_iter: u32,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long)]
    seed: Option<u64>,
    #[arg(long, default_value_t = 0)]
    fund_seed: u128,
    #[arg(long, default_value_t = u64::MAX)]
    slots: u64,
}
```

- [ ] **Step 2: Add helper to collect data files from a directory**

Add this function after the existing `gcd` and `rational_from_float` helpers:

```rust
fn collect_data_files(args: &Args) -> Vec<PathBuf> {
    if let Some(dir) = &args.data_dir {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .expect("failed to read data directory")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |ext| ext == "csv"))
            .collect();
        files.sort();
        if files.is_empty() {
            eprintln!("  ERROR: no CSV files found in {}", dir.display());
            std::process::exit(1);
        }
        eprintln!("  found {} data files in {}", files.len(), dir.display());
        for f in &files {
            eprintln!("    - {}", f.file_name().unwrap_or_default().to_string_lossy());
        }
        files
    } else if let Some(path) = &args.data {
        vec![path.clone()]
    } else {
        eprintln!("  ERROR: must provide --data or --data-dir");
        std::process::exit(1);
    }
}
```

- [ ] **Step 3: Update run_sim to accept a slice of data files with min-scoring**

Replace the existing `run_sim` function with:

```rust
fn run_sim_single(data_path: &PathBuf, params: PremiumParams, budget_cap: f64, fund_seed: u128, max_slots: u64) -> f64 {
    let init_price: u64 = 50_000 * POS_SCALE as u64;
    let vault_seed: u128 = 10_000_000_000;

    let mut engine = SimEngine::new(params, 400, 100);
    engine.initialize(init_price, vault_seed, fund_seed);
    let fund_start = engine.fund_balance();

    let mut source = match BinanceTradeSource::from_path(data_path) {
        Ok(s) => s,
        Err(_) => return f64::NEG_INFINITY,
    };

    while let Some(event) = source.next_event() {
        if engine.clock.current_slot() >= max_slots {
            break;
        }
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

    let haircuts = engine.metrics.haircut_activations();
    let surplus = if fund_end >= fund_start { fund_end - fund_start } else { 0 };
    let base_score = surplus as f64 / total_notional as f64;
    base_score * (0.5_f64).powi(haircuts as i32)
}

fn run_sim_multi(data_files: &[PathBuf], params: PremiumParams, budget_cap: f64, fund_seed: u128, max_slots: u64) -> f64 {
    let mut min_score = f64::INFINITY;
    for path in data_files {
        let score = run_sim_single(path, params, budget_cap, fund_seed, max_slots);
        if score == f64::NEG_INFINITY {
            return f64::NEG_INFINITY;
        }
        if score < min_score {
            min_score = score;
        }
    }
    if min_score == f64::INFINITY {
        return 0.0;
    }
    min_score
}
```

- [ ] **Step 4: Update main() to use multi-day scoring and export SimConfig**

Replace `main()` with:

```rust
fn main() {
    let args = Args::parse();
    let bounds = default_param_bounds();
    let data_files = collect_data_files(&args);
    let num_files = data_files.len();

    eprintln!("  starting optimizer: {} max iterations, {} data files, min-scoring", args.max_iter, num_files);

    let budget = args.budget_cap;
    let fund_seed = args.fund_seed;
    let max_slots = args.slots;

    let result = nelder_mead(
        &bounds,
        |p| run_sim_multi(&data_files, params_from_vec(p), budget, fund_seed, max_slots),
        args.max_iter,
        50,
        args.seed,
    );

    let best_params = params_from_vec(&result.best_params);
    eprintln!("  optimizer done: {} iterations, {:.0}s elapsed, best score = {:.10}", result.iterations, result.elapsed_secs, result.best_score);
    eprintln!("  best params: {:?}", result.best_params);

    // Export SimConfig
    let sim_config = SimConfig {
        premium_params: best_params,
        fund_seed,
        budget_cap: budget,
    };
    let config_path = PathBuf::from("output/sim-config.json");
    sim_config.save(&config_path).expect("failed to save sim-config.json");
    eprintln!("  config saved to {}", config_path.display());

    // Run final replay on first data file for the report
    let report_data = data_files.first().unwrap();
    let init_price: u64 = 50_000 * POS_SCALE as u64;
    let mut engine = SimEngine::new(best_params, 400, 100);
    engine.initialize(init_price, 10_000_000_000, fund_seed);
    let fund_start = engine.fund_balance();

    if let Ok(mut source) = BinanceTradeSource::from_path(report_data) {
        while let Some(event) = source.next_event() {
            if engine.clock.current_slot() >= max_slots {
                break;
            }
            let _ = engine.process_event(&event);
        }
    }

    let config = ReportConfig {
        scenario_name: format!("optimize-best-{}", result.iterations),
        params: engine.engine.premium_params,
        budget_cap_pct: budget,
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

- [ ] **Step 5: Build and verify**

```bash
cd /home/acheron28nyx/percolator && cargo build --bin sim-optimize
```

Expected: compiles without errors.

- [ ] **Step 6: Commit**

```bash
git add percolator-sim/bin/sim_optimize.rs
git commit -m "feat(sim): add multi-day min-scoring optimizer with --data-dir and SimConfig export"
```

---

### Task 6: Fix sim-live Binary

**Files:**
- Modify: `percolator-sim/bin/sim_live.rs`

- [ ] **Step 1: Update imports, CLI args, and add dynamic price initialization**

Replace the entire `sim_live.rs` with:

```rust
use clap::Parser;
use percolator_insurance::PremiumParams;
use percolator_sim::config::SimConfig;
use percolator_sim::engine::SimEngine;
use percolator_sim::feed::binance_ws::connect_binance_trades;
use percolator_sim::metrics::report::{generate_report, write_report, ReportConfig};
use percolator_sim::{MarketEvent, POS_SCALE};
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
    #[arg(long, group = "param_source")]
    params: Option<PathBuf>,
    #[arg(long, group = "param_source")]
    config: Option<PathBuf>,
    #[arg(long, default_value_t = 3600)]
    duration: u64,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long, default_value_t = 0.1)]
    budget_cap: f64,
    #[arg(long, default_value_t = 0)]
    fund_seed: u128,
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
    let matches = <Args as clap::CommandFactory>::command().get_matches();

    let (params, fund_seed, budget_cap) = if let Some(config_path) = &args.config {
        let cfg = SimConfig::load(config_path).expect("failed to load config");
        let fs = if matches.value_source("fund_seed") == Some(clap::parser::ValueSource::CommandLine) {
            args.fund_seed
        } else {
            cfg.fund_seed
        };
        let bc = if matches.value_source("budget_cap") == Some(clap::parser::ValueSource::CommandLine) {
            args.budget_cap
        } else {
            cfg.budget_cap
        };
        (cfg.premium_params, fs, bc)
    } else if let Some(params_path) = &args.params {
        let json = std::fs::read_to_string(params_path).expect("failed to read params");
        let p: PremiumParams = serde_json::from_str(&json).expect("failed to parse params");
        (p, args.fund_seed, args.budget_cap)
    } else {
        (default_premium_params(), args.fund_seed, args.budget_cap)
    };

    let (tx, mut rx) = mpsc::channel(10_000);

    let symbol = args.symbol.clone();
    tokio::spawn(async move {
        if let Err(e) = connect_binance_trades(&symbol, tx).await {
            eprintln!("  websocket error: {}", e);
        }
    });

    eprintln!("  connecting to Binance {}...", args.symbol);

    // Wait for first trade to get real market price
    let first_event = match timeout(Duration::from_secs(30), rx.recv()).await {
        Ok(Some(event)) => event,
        Ok(None) => {
            eprintln!("  ERROR: websocket closed before receiving any data");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("  ERROR: timed out waiting for first trade (30s)");
            std::process::exit(1);
        }
    };

    let init_price = match &first_event {
        MarketEvent::Trade { price, .. } => *price,
        _ => {
            eprintln!("  ERROR: first event was not a trade");
            std::process::exit(1);
        }
    };

    eprintln!("  first trade price: {} (raw), initializing engine...",
        init_price as f64 / POS_SCALE as f64);

    let mut engine = SimEngine::new(params, 400, 100);
    engine.initialize(init_price, 10_000_000_000, fund_seed);
    let fund_start = engine.fund_balance();

    // Process the first event
    let _ = engine.process_event(&first_event);
    let mut event_count = 1u64;

    eprintln!("  running for {}s...", args.duration);

    let deadline = Duration::from_secs(args.duration);
    let _ = timeout(deadline, async {
        while let Some(event) = rx.recv().await {
            let _ = engine.process_event(&event);
            event_count += 1;
            if event_count % 1000 == 0 {
                eprint!("\r  {} events, slot {}, fund: {}",
                    event_count, engine.clock.current_slot(), engine.fund_balance());
            }
        }
    }).await;

    eprintln!("\n  done: {} events, {} slots", event_count, engine.clock.current_slot());
    eprintln!("  conservation check: {}", if engine.conservation_ok() { "PASS" } else { "FAIL" });

    let config = ReportConfig {
        scenario_name: format!("live-{}", args.symbol),
        params: engine.engine.premium_params,
        budget_cap_pct: budget_cap,
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

- [ ] **Step 2: Build and verify**

```bash
cd /home/acheron28nyx/percolator && cargo build --bin sim-live
```

Expected: compiles without errors.

- [ ] **Step 3: Commit**

```bash
git add percolator-sim/bin/sim_live.rs
git commit -m "feat(sim): fix sim-live — dynamic init price, --config/--fund-seed flags"
```

---

### Task 7: Run Full Test Suite

**Files:** None (verification only)

- [ ] **Step 1: Run all workspace tests**

```bash
cd /home/acheron28nyx/percolator && cargo test --workspace 2>&1 | grep "test result:"
```

Expected: all test suites pass (464+ tests), zero failures. The new `config` module adds 3 tests.

- [ ] **Step 2: Verify new config tests specifically**

```bash
cargo test -p percolator-sim config:: -- --nocapture
```

Expected: `roundtrip_json`, `save_and_load`, `load_nonexistent_file_errors` — 3 passed.

---

### Task 8: Run Multi-Day Optimizer

**Files:** None (execution only, produces output files)

- [ ] **Step 1: Launch the optimizer on 7-day dataset**

```bash
cd /home/acheron28nyx/percolator && cargo run --release --bin sim-optimize -- \
    --data-dir data/ \
    --fund-seed 50000000000 \
    --max-iter 200 \
    --slots 216000 \
    --output output/optimize-multiday-20260504.txt
```

This will take a long time (~20+ hours for 200 iterations × 7 days × ~55s/day). Run in background.

Expected output: `output/optimize-multiday-20260504.txt` (report) and `output/sim-config.json` (best params).

- [ ] **Step 2: Check optimizer convergence**

After completion, verify:

```bash
tail -40 output/optimize-multiday-20260504.txt
cat output/sim-config.json
```

Check that:
- `leverage_exponent_num / leverage_exponent_den` ≥ 1.5
- `min_commitment_slots` ≤ 2700
- Haircut activations ≤ 5
- Budget status is UNDER

---

### Task 9: Validate Best Params on Each Day

**Files:** None (execution only)

- [ ] **Step 1: Replay each day individually with optimized params**

```bash
cd /home/acheron28nyx/percolator
for f in data/BTCUSDT-aggTrades-2026-04-2*.csv; do
    DAY=$(basename "$f" .csv | tail -c 11)
    echo "=== Replaying $DAY ==="
    cargo run --release --bin sim-replay -- \
        --data "$f" \
        --config output/sim-config.json \
        --output "output/validate-multiday-${DAY}.txt"
done
```

- [ ] **Step 2: Compare results across days**

```bash
for f in output/validate-multiday-*.txt; do
    echo "=== $(basename $f) ==="
    grep -E "(Haircut activations|Budget status|End balance|VERDICT)" "$f"
    echo
done
```

Expected: every day shows ≤ 5 haircuts, budget UNDER, and ideally a PASS verdict.

---

### Task 10: Smoke Test sim-live (10 minutes)

**Files:** None (execution only)

- [ ] **Step 1: Run 10-minute live test**

```bash
cd /home/acheron28nyx/percolator && cargo run --release --bin sim-live -- \
    --symbol BTCUSDT \
    --duration 600 \
    --config output/sim-config.json \
    --output output/live-smoke-20260504.txt
```

- [ ] **Step 2: Verify the smoke test output**

Check:
- "connecting to Binance" message appeared
- "first trade price: XXX" shows a reasonable BTC price
- Event count > 0
- Conservation check: PASS
- Report file generated at `output/live-smoke-20260504.txt`

```bash
cat output/live-smoke-20260504.txt
```

---

### Task 11: Extended sim-live Run (1 hour, background)

**Files:** None (execution only)

- [ ] **Step 1: Launch 1-hour live run in background**

```bash
cd /home/acheron28nyx/percolator && cargo run --release --bin sim-live -- \
    --symbol BTCUSDT \
    --duration 3600 \
    --config output/sim-config.json \
    --output output/live-1h-20260504.txt \
    2>output/live-1h-20260504.log &
```

- [ ] **Step 2: After completion, review the report**

```bash
cat output/live-1h-20260504.txt
grep -E "(Haircut|Budget|VERDICT|events)" output/live-1h-20260504.log
```

Check: haircut count, premium ratio vs budget, fund growth, conservation. Compare against the optimizer's best-day replay to confirm the live pipeline produces comparable results.
