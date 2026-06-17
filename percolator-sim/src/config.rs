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
                // Disabled: preserve pre-existing sim economics (opt in to price these later)
                volatility_mult_num: 1_000,
                volatility_mult_den: 1_000,
                leverage_tail_threshold_bps: 10_000,
                leverage_tail_steepness: 0,
                collection_maint_buffer_bps: 0,
                max_oracle_deviation_bps: 0,
                max_oracle_staleness_slots: 0,
                require_authorization: false,
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
