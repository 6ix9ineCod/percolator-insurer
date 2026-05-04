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
    let mut engine = SimEngine::new(params, 400, 100);
    engine.initialize(init_price, 10_000_000_000, 0);
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
                eprint!("\r  {} events, slot {}, toxicity: {}",
                    event_count, engine.clock.current_slot(), engine.signal.toxicity(0));
            }
        }
    }).await;

    eprintln!("\n  done: {} events", event_count);

    let config = ReportConfig {
        scenario_name: format!("live-{}", args.symbol),
        params: engine.engine.premium_params,
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
