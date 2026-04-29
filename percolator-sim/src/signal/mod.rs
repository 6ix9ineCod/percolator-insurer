pub mod volume;
pub mod depth;
pub mod aggression;

pub use volume::VolumeImbalance;
pub use depth::DepthThinning;
pub use aggression::TradeAggression;

pub struct FlowSignal {
    pub volume: VolumeImbalance,
    pub depth: DepthThinning,
    pub aggression: TradeAggression,
}

impl FlowSignal {
    pub fn new() -> Self {
        Self {
            volume: VolumeImbalance::new(),
            depth: DepthThinning::new(10),
            aggression: TradeAggression::new(),
        }
    }

    pub fn toxicity(&self, now_ms: u64) -> u8 {
        let v = self.volume.score(now_ms) as u32;
        let d = self.depth.score() as u32;
        let a = self.aggression.score(now_ms) as u32;
        let composite = (v * 40 + d * 30 + a * 30) / 100;
        composite.min(100) as u8
    }

    pub fn gc(&mut self, now_ms: u64) {
        self.volume.gc(now_ms);
        self.aggression.gc(now_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_all_zero() {
        let fs = FlowSignal::new();
        assert_eq!(fs.toxicity(1000), 0);
    }

    #[test]
    fn composite_weighted_correctly() {
        let mut fs = FlowSignal::new();
        fs.volume.record_trade(1000, 100, true);
        let t = fs.toxicity(1000);
        assert_eq!(t, 40);
    }

    #[test]
    fn composite_all_max() {
        let mut fs = FlowSignal::new();
        fs.volume.record_trade(1000, 100, true);
        fs.aggression.record(1000, 100, true);
        let bids = vec![(100, 100)];
        let asks = vec![(101, 100)];
        fs.depth.update(&bids, &asks);
        let bids2 = vec![(100, 50)];
        let asks2 = vec![(101, 50)];
        fs.depth.update(&bids2, &asks2);
        assert_eq!(fs.toxicity(1000), 100);
    }
}
