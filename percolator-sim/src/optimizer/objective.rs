pub struct ObjectiveResult {
    pub fund_surplus: u128,
    pub total_notional: u128,
    pub total_premiums: u128,
    pub budget_cap: f64,
}

impl ObjectiveResult {
    pub fn score(&self) -> f64 {
        if self.total_notional == 0 {
            return 0.0;
        }
        let premium_ratio = self.total_premiums as f64 / self.total_notional as f64;
        if premium_ratio > self.budget_cap {
            return f64::NEG_INFINITY;
        }
        self.fund_surplus as f64 / self.total_notional as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feasible_positive_surplus() {
        let r = ObjectiveResult {
            fund_surplus: 1000,
            total_notional: 1_000_000,
            total_premiums: 500,
            budget_cap: 0.001,
        };
        let score = r.score();
        assert!(score > 0.0);
    }

    #[test]
    fn infeasible_returns_neg_infinity() {
        let r = ObjectiveResult {
            fund_surplus: 1000,
            total_notional: 1_000_000,
            total_premiums: 2000,
            budget_cap: 0.001,
        };
        assert_eq!(r.score(), f64::NEG_INFINITY);
    }

    #[test]
    fn zero_notional_returns_zero() {
        let r = ObjectiveResult {
            fund_surplus: 0,
            total_notional: 0,
            total_premiums: 0,
            budget_cap: 0.001,
        };
        assert_eq!(r.score(), 0.0);
    }
}
