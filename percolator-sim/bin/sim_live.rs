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

    eprintln!("  first trade price: {:.2} USD, initializing engine...",
        init_price as f64 / POS_SCALE as f64);

    let mut engine = SimEngine::new(params, 400, 100);
    engine.initialize(init_price, 10_000_000_000, fund_seed);
    let fund_start = engine.fund_balance();

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
