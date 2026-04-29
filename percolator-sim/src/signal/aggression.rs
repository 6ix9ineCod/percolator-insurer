const WINDOW_5S: u64 = 5_000;

struct AggressionRecord {
    timestamp_ms: u64,
    qty: u64,
    is_buy: bool,
}

pub struct TradeAggression {
    trades: Vec<AggressionRecord>,
}

impl TradeAggression {
    pub fn new() -> Self {
        Self { trades: Vec::new() }
    }

    pub fn record(&mut self, timestamp_ms: u64, qty: u64, is_buy: bool) {
        self.trades.push(AggressionRecord { timestamp_ms, qty, is_buy });
    }

    pub fn score(&self, now_ms: u64) -> u8 {
        let cutoff = now_ms.saturating_sub(WINDOW_5S);
        let mut buy_vol: u64 = 0;
        let mut sell_vol: u64 = 0;
        for t in &self.trades {
            if t.timestamp_ms >= cutoff && t.timestamp_ms <= now_ms {
                if t.is_buy {
                    buy_vol = buy_vol.saturating_add(t.qty);
                } else {
                    sell_vol = sell_vol.saturating_add(t.qty);
                }
            }
        }
        let total = buy_vol.saturating_add(sell_vol);
        if total == 0 {
            return 0;
        }
        let dominant = buy_vol.max(sell_vol);
        let ratio_x1000 = (dominant as u128 * 1000) / total as u128;
        let score = (ratio_x1000 as i64 - 500) * 200 / 1000;
        (score.max(0) as u8).min(100)
    }

    pub fn gc(&mut self, now_ms: u64) {
        let cutoff = now_ms.saturating_sub(WINDOW_5S);
        self.trades.retain(|t| t.timestamp_ms >= cutoff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_aggression_score_zero() {
        let mut ta = TradeAggression::new();
        ta.record(1000, 100, true);
        ta.record(1000, 100, false);
        assert_eq!(ta.score(1000), 0);
    }

    #[test]
    fn fully_buy_aggressive_score_100() {
        let mut ta = TradeAggression::new();
        ta.record(1000, 100, true);
        assert_eq!(ta.score(1000), 100);
    }

    #[test]
    fn fully_sell_aggressive_score_100() {
        let mut ta = TradeAggression::new();
        ta.record(1000, 100, false);
        assert_eq!(ta.score(1000), 100);
    }

    #[test]
    fn no_trades_score_zero() {
        let ta = TradeAggression::new();
        assert_eq!(ta.score(5000), 0);
    }

    #[test]
    fn seventy_five_twenty_five_split() {
        let mut ta = TradeAggression::new();
        ta.record(1000, 75, true);
        ta.record(1000, 25, false);
        assert_eq!(ta.score(1000), 50);
    }

    #[test]
    fn old_trades_expire() {
        let mut ta = TradeAggression::new();
        ta.record(1000, 100, true);
        ta.record(7000, 50, false);
        let s = ta.score(7000);
        assert_eq!(s, 100);
    }
}
