pub struct RateLimiter {
    max_fraction: f64,
}

impl RateLimiter {
    pub fn new(max_fraction: f64) -> Self {
        Self { max_fraction }
    }

    pub fn limit(&self, current: f64, proposed: f64) -> f64 {
        if current == 0.0 {
            return proposed;
        }
        let max_delta = current.abs() * self.max_fraction;
        let delta = proposed - current;
        if delta.abs() <= max_delta {
            proposed
        } else {
            current + delta.signum() * max_delta
        }
    }
}

pub fn apply_rate_limits(current: &[f64], proposed: &[f64], max_fraction: f64) -> Vec<f64> {
    let rl = RateLimiter::new(max_fraction);
    current.iter().zip(proposed.iter())
        .map(|(&c, &p)| rl.limit(c, p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_limit_unchanged() {
        let rl = RateLimiter::new(0.10);
        assert_eq!(rl.limit(100.0, 105.0), 105.0);
    }

    #[test]
    fn exceeds_limit_clamped_up() {
        let rl = RateLimiter::new(0.10);
        assert_eq!(rl.limit(100.0, 120.0), 110.0);
    }

    #[test]
    fn exceeds_limit_clamped_down() {
        let rl = RateLimiter::new(0.10);
        assert_eq!(rl.limit(100.0, 80.0), 90.0);
    }

    #[test]
    fn zero_current_no_panic() {
        let rl = RateLimiter::new(0.10);
        assert_eq!(rl.limit(0.0, 50.0), 50.0);
    }
}
