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
        ParamBounds::new(10.0, 1000.0),       // base_rate_per_slot
        ParamBounds::new(1.0, 3.0),           // leverage_exponent_num
        ParamBounds::new(1.0, 2.0),           // leverage_exponent_den
        ParamBounds::new(54000.0, 432000.0),  // min_commitment_slots
        ParamBounds::new(2000.0, 8000.0),     // crowding_cap
        ParamBounds::new(1500.0, 5000.0),     // oi_vault_mult_max
        ParamBounds::new(2000.0, 10000.0),    // pool_health_mult_max
        ParamBounds::new(1.0, 100.0),         // min_premium_per_slot
    ]
}

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
