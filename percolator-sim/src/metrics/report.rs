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
