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

#[test]
fn full_replay_synthetic_data() {
    let price: u64 = 50_000 * POS_SCALE as u64;
    let mut engine = SimEngine::new(test_premium_params(), 400, 100);
    engine.initialize(price, 10_000_000_000, 0);

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
    engine.initialize(price, 10_000_000_000, 0);
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
