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
        ParamBounds::new(10.0, 1000.0),       // [0] base_rate_per_slot
        ParamBounds::new(1.3, 3.0),           // [1] leverage_exponent (floor 1.3 → effective 1.5 with denom 4)
        ParamBounds::new(100.0, 2700.0),      // [2] min_commitment_slots (40ms – 18min at 400ms slots)
        ParamBounds::new(2000.0, 8000.0),     // [3] crowding_cap
        ParamBounds::new(1500.0, 5000.0),     // [4] oi_vault_mult_max
        ParamBounds::new(2000.0, 10000.0),    // [5] pool_health_mult_max
        ParamBounds::new(1.0, 100.0),         // [6] min_premium_per_slot
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
        assert_eq!(bounds.len(), 7);
        for b in &bounds {
            assert!(b.min < b.max);
        }
    }
}
