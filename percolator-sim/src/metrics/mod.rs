pub mod report;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub slot: u64,
    pub timestamp_ms: u64,
    pub insurance_fund_balance: u128,
    pub pool_balance: u128,
    pub pool_total_collected: u128,
    pub pool_total_paid_out: u128,
    pub haircut_num: u128,
    pub haircut_den: u128,
    pub vault_balance: u128,
    pub total_oi_long: u128,
    pub total_oi_short: u128,
    pub active_accounts: u32,
    pub flow_toxicity: u8,
}

#[derive(Clone, Debug)]
struct LiquidationEvent {
    slot: u64,
    capital: u128,
}

pub struct MetricsCollector {
    pub snapshots: Vec<Snapshot>,
    pub sample_interval: u64,
    pub liquidation_count: u64,
    pub capital_liquidated: u128,
    pub total_notional_traded: u128,
    liquidations: Vec<LiquidationEvent>,
}

impl MetricsCollector {
    pub fn new(sample_interval: u64) -> Self {
        Self {
            snapshots: Vec::new(),
            sample_interval,
            liquidation_count: 0,
            capital_liquidated: 0,
            total_notional_traded: 0,
            liquidations: Vec::new(),
        }
    }

    pub fn record(&mut self, snapshot: Snapshot) {
        self.snapshots.push(snapshot);
    }

    pub fn record_liquidation(&mut self, slot: u64, capital: u128) {
        self.liquidation_count += 1;
        self.capital_liquidated = self.capital_liquidated.saturating_add(capital);
        self.liquidations.push(LiquidationEvent { slot, capital });
    }

    pub fn record_trade_notional(&mut self, notional: u128) {
        self.total_notional_traded = self.total_notional_traded.saturating_add(notional);
    }

    pub fn haircut_activations(&self) -> u64 {
        let mut count = 0u64;
        let mut was_active = false;
        for s in &self.snapshots {
            let active = s.haircut_num < s.haircut_den;
            if active && !was_active {
                count += 1;
            }
            was_active = active;
        }
        count
    }

    pub fn haircut_slots(&self) -> u64 {
        self.snapshots.iter().filter(|s| s.haircut_num < s.haircut_den).count() as u64
    }

    pub fn count_cascades(&self, window_slots: u64) -> (u64, u64) {
        if self.liquidations.len() < 4 {
            return (0, 0);
        }
        let mut cascade_count = 0u64;
        let mut largest = 0u64;
        let liqs = &self.liquidations;
        let mut i = 0;
        while i < liqs.len() {
            let start_slot = liqs[i].slot;
            let mut j = i + 1;
            while j < liqs.len() && liqs[j].slot <= start_slot + window_slots {
                j += 1;
            }
            let group_size = (j - i) as u64;
            if group_size > 3 {
                cascade_count += 1;
                if group_size > largest {
                    largest = group_size;
                }
            }
            i = j;
        }
        (cascade_count, largest)
    }

    pub fn fund_min(&self) -> (u128, u64) {
        self.snapshots.iter()
            .map(|s| (s.insurance_fund_balance, s.slot))
            .min_by_key(|&(b, _)| b)
            .unwrap_or((0, 0))
    }

    pub fn fund_max(&self) -> (u128, u64) {
        self.snapshots.iter()
            .map(|s| (s.insurance_fund_balance, s.slot))
            .max_by_key(|&(b, _)| b)
            .unwrap_or((0, 0))
    }

    pub fn deficit_slots(&self) -> u64 {
        self.snapshots.iter().filter(|s| s.insurance_fund_balance == 0).count() as u64
    }

    pub fn avg_toxicity(&self) -> u8 {
        if self.snapshots.is_empty() {
            return 0;
        }
        let sum: u64 = self.snapshots.iter().map(|s| s.flow_toxicity as u64).sum();
        (sum / self.snapshots.len() as u64) as u8
    }

    pub fn max_toxicity(&self) -> (u8, u64) {
        self.snapshots.iter()
            .map(|s| (s.flow_toxicity, s.slot))
            .max_by_key(|&(t, _)| t)
            .unwrap_or((0, 0))
    }

    pub fn toxicity_above_threshold(&self, threshold: u8) -> u64 {
        self.snapshots.iter().filter(|s| s.flow_toxicity > threshold).count() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_collector_empty() {
        let mc = MetricsCollector::new(100);
        assert_eq!(mc.snapshots.len(), 0);
        assert_eq!(mc.liquidation_count, 0);
    }

    #[test]
    fn record_snapshot() {
        let mut mc = MetricsCollector::new(100);
        mc.record(Snapshot {
            slot: 100, timestamp_ms: 40000, insurance_fund_balance: 5000,
            pool_balance: 1000, pool_total_collected: 1200, pool_total_paid_out: 200,
            haircut_num: 1, haircut_den: 1, vault_balance: 100000,
            total_oi_long: 50000, total_oi_short: 45000, active_accounts: 10,
            flow_toxicity: 35,
        });
        assert_eq!(mc.snapshots.len(), 1);
    }

    #[test]
    fn record_liquidation() {
        let mut mc = MetricsCollector::new(100);
        mc.record_liquidation(50, 10000);
        mc.record_liquidation(55, 5000);
        assert_eq!(mc.liquidation_count, 2);
        assert_eq!(mc.capital_liquidated, 15000);
    }

    #[test]
    fn cascade_detection() {
        let mut mc = MetricsCollector::new(100);
        mc.record_liquidation(100, 1000);
        mc.record_liquidation(120, 1000);
        mc.record_liquidation(150, 1000);
        mc.record_liquidation(180, 1000);
        let cascades = mc.count_cascades(100);
        assert_eq!(cascades.0, 1);
        assert_eq!(cascades.1, 4);
    }

    #[test]
    fn no_cascade_when_spread_out() {
        let mut mc = MetricsCollector::new(100);
        mc.record_liquidation(100, 1000);
        mc.record_liquidation(300, 1000);
        mc.record_liquidation(500, 1000);
        let cascades = mc.count_cascades(100);
        assert_eq!(cascades.0, 0);
    }

    #[test]
    fn haircut_count() {
        let mut mc = MetricsCollector::new(100);
        mc.record(Snapshot {
            slot: 0, timestamp_ms: 0, insurance_fund_balance: 1000,
            pool_balance: 500, pool_total_collected: 500, pool_total_paid_out: 0,
            haircut_num: 1, haircut_den: 1, vault_balance: 10000,
            total_oi_long: 5000, total_oi_short: 5000, active_accounts: 5,
            flow_toxicity: 0,
        });
        mc.record(Snapshot {
            slot: 100, timestamp_ms: 40000, insurance_fund_balance: 0,
            pool_balance: 0, pool_total_collected: 500, pool_total_paid_out: 500,
            haircut_num: 9, haircut_den: 10, vault_balance: 10000,
            total_oi_long: 5000, total_oi_short: 5000, active_accounts: 5,
            flow_toxicity: 80,
        });
        mc.record(Snapshot {
            slot: 200, timestamp_ms: 80000, insurance_fund_balance: 100,
            pool_balance: 100, pool_total_collected: 600, pool_total_paid_out: 500,
            haircut_num: 1, haircut_den: 1, vault_balance: 10000,
            total_oi_long: 5000, total_oi_short: 5000, active_accounts: 5,
            flow_toxicity: 20,
        });
        assert_eq!(mc.haircut_activations(), 1);
        assert_eq!(mc.haircut_slots(), 1);
    }
}
