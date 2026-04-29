struct TradeRecord {
    timestamp_ms: u64,
    qty: u64,
    is_buy: bool,
}

pub struct VolumeImbalance {
    trades: Vec<TradeRecord>,
}

const WINDOW_1S: u64 = 1_000;
const WINDOW_5S: u64 = 5_000;
const WINDOW_30S: u64 = 30_000;

impl VolumeImbalance {
    pub fn new() -> Self {
        Self { trades: Vec::new() }
    }

    pub fn record_trade(&mut self, timestamp_ms: u64, qty: u64, is_buy: bool) {
        self.trades.push(TradeRecord { timestamp_ms, qty, is_buy });
    }

    pub fn score(&self, now_ms: u64) -> u8 {
        let s1 = self.window_score(now_ms, WINDOW_1S);
        let s5 = self.window_score(now_ms, WINDOW_5S);
        let s30 = self.window_score(now_ms, WINDOW_30S);
        s1.max(s5).max(s30)
    }

    fn window_score(&self, now_ms: u64, window_ms: u64) -> u8 {
        let cutoff = now_ms.saturating_sub(window_ms);
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
        let diff = if buy_vol > sell_vol {
            buy_vol - sell_vol
        } else {
            sell_vol - buy_vol
        };
        ((diff as u128 * 100) / total as u128) as u8
    }

    pub fn gc(&mut self, now_ms: u64) {
        let cutoff = now_ms.saturating_sub(WINDOW_30S);
        self.trades.retain(|t| t.timestamp_ms >= cutoff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_volume_score_zero() {
        let mut vi = VolumeImbalance::new();
        vi.record_trade(1000, 100, true);
        vi.record_trade(1000, 100, false);
        assert_eq!(vi.score(1000), 0);
    }

    #[test]
    fn fully_one_sided_buy_score_100() {
        let mut vi = VolumeImbalance::new();
        vi.record_trade(1000, 100, true);
        assert_eq!(vi.score(1000), 100);
    }

    #[test]
    fn fully_one_sided_sell_score_100() {
        let mut vi = VolumeImbalance::new();
        vi.record_trade(1000, 100, false);
        assert_eq!(vi.score(1000), 100);
    }

    #[test]
    fn no_trades_score_zero() {
        let vi = VolumeImbalance::new();
        assert_eq!(vi.score(5000), 0);
    }

    #[test]
    fn two_to_one_imbalance() {
        let mut vi = VolumeImbalance::new();
        vi.record_trade(1000, 200, true);
        vi.record_trade(1000, 100, false);
        let s = vi.score(1000);
        assert!(s >= 30 && s <= 36, "expected ~33, got {}", s);
    }

    #[test]
    fn old_trades_expire_from_1s_window() {
        let mut vi = VolumeImbalance::new();
        vi.record_trade(1000, 100, true);
        vi.record_trade(3000, 100, false);
        let s = vi.score(3000);
        assert_eq!(s, 100);
    }

    #[test]
    fn max_of_windows_used() {
        let mut vi = VolumeImbalance::new();
        vi.record_trade(1000, 500, true);
        vi.record_trade(29000, 100, false);
        let s = vi.score(29000);
        assert_eq!(s, 100);
    }
}
