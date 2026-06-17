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
        // Disabled: preserve pre-existing sim economics (opt in to price these later)
        volatility_mult_num: 1_000,
        volatility_mult_den: 1_000,
        leverage_tail_threshold_bps: 10_000,
        leverage_tail_steepness: 0,
        collection_maint_buffer_bps: 0,
        max_oracle_deviation_bps: 0,
        max_oracle_staleness_slots: 0,
        require_authorization: false,
    }
}

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
        params: engine.engine.premium_params,
        budget_cap_pct: budget_cap,
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
