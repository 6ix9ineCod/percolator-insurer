use clap::Parser;
use percolator_insurance::PremiumParams;
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

    let init_price: u64 = 50_000 * POS_SCALE as u64;
    let mut engine = SimEngine::new(best_params, 400, 100);
    engine.initialize(init_price, 10_000_000_000);
    let fund_start = engine.fund_balance();

    if let Ok(mut source) = BinanceTradeSource::from_path(&args.data) {
        while let Some(event) = source.next_event() {
            let _ = engine.process_event(&event);
        }
    }

    let config = ReportConfig {
        scenario_name: format!("optimize-best-{}", result.iterations),
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
        PathBuf::from(format!("output/optimize-{}.txt", timestamp))
    });
    write_report(&report, &output_path).expect("failed to write report");
    eprintln!("  report saved to {}", output_path.display());
}
