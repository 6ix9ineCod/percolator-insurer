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

fn rational_from_float(x: f64) -> (u64, u64) {
    let denom = 4u64;
    let numer = (x * denom as f64).round().max(1.0) as u64;
    let g = gcd(numer, denom);
    (numer / g, denom / g)
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

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

fn params_from_vec(v: &[f64]) -> PremiumParams {
    let (exp_num, exp_den) = rational_from_float(v[1]);
    PremiumParams {
        base_rate_per_slot: v[0].round().max(1.0) as u128,
        leverage_exponent_num: exp_num,
        leverage_exponent_den: exp_den,
        min_commitment_slots: v[2].round().max(1.0) as u64,
        crowding_low_ratio_num: 1500,
        crowding_low_ratio_den: 1000,
        crowding_high_ratio_num: 5000,
        crowding_high_ratio_den: 1000,
        crowding_cap: v[3].round().max(1.0) as u128,
        oi_vault_floor_ratio_num: 1,
        oi_vault_floor_ratio_den: 1,
        oi_vault_cap_ratio_num: 5,
        oi_vault_cap_ratio_den: 1,
        oi_vault_mult_max: v[4].round().max(1.0) as u128,
        pool_health_low_num: 1,
        pool_health_low_den: 100,
        pool_health_high_num: 5,
        pool_health_high_den: 100,
        pool_health_mult_max: v[5].round().max(1.0) as u128,
        min_premium_per_slot: v[6].round().max(1.0) as u128,
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

fn main() {
    let args = Args::parse();
    let bounds = default_param_bounds();
    let data_files = collect_data_files(&args);
    let num_files = data_files.len();

    eprintln!("  starting optimizer: {} max iterations, {} data file(s), min-scoring",
        args.max_iter, num_files);

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
    eprintln!("  optimizer done: {} iterations, {:.0}s elapsed, best score = {:.10}",
        result.iterations, result.elapsed_secs, result.best_score);
    eprintln!("  best params: {:?}", result.best_params);

    let sim_config = SimConfig {
        premium_params: best_params,
        fund_seed,
        budget_cap: budget,
    };
    let config_path = PathBuf::from("output/sim-config.json");
    sim_config.save(&config_path).expect("failed to save sim-config.json");
    eprintln!("  config saved to {}", config_path.display());

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
