pub struct DepthThinning {
    top_n: usize,
    prev_depth: u64,
    curr_depth: u64,
    has_prev: bool,
}

impl DepthThinning {
    pub fn new(top_n: usize) -> Self {
        Self {
            top_n,
            prev_depth: 0,
            curr_depth: 0,
            has_prev: false,
        }
    }

    pub fn update(&mut self, bids: &[(u64, u64)], asks: &[(u64, u64)]) {
        let bid_sum: u64 = bids.iter().take(self.top_n).map(|(_, q)| q).sum();
        let ask_sum: u64 = asks.iter().take(self.top_n).map(|(_, q)| q).sum();
        let total = bid_sum.saturating_add(ask_sum);

        if self.has_prev {
            self.prev_depth = self.curr_depth;
        }
        self.curr_depth = total;
        self.has_prev = true;
    }

    pub fn score(&self) -> u8 {
        if !self.has_prev || self.prev_depth == 0 {
            return 0;
        }
        if self.curr_depth >= self.prev_depth {
            return 0;
        }
        let diff = self.prev_depth - self.curr_depth;
        let raw = (diff as u128 * 200) / self.prev_depth as u128;
        raw.min(100) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_previous_snapshot_score_zero() {
        let dt = DepthThinning::new(10);
        assert_eq!(dt.score(), 0);
    }

    #[test]
    fn no_change_score_zero() {
        let mut dt = DepthThinning::new(10);
        let levels = vec![(100, 50), (99, 30), (98, 20)];
        dt.update(&levels, &levels);
        assert_eq!(dt.score(), 0);
    }

    #[test]
    fn fifty_percent_thinning() {
        let mut dt = DepthThinning::new(10);
        let bids = vec![(100, 100)];
        let asks = vec![(101, 100)];
        dt.update(&bids, &asks);
        let bids2 = vec![(100, 50)];
        let asks2 = vec![(101, 50)];
        dt.update(&bids2, &asks2);
        assert_eq!(dt.score(), 100);
    }

    #[test]
    fn twenty_five_percent_thinning() {
        let mut dt = DepthThinning::new(10);
        let bids = vec![(100, 100)];
        let asks = vec![(101, 100)];
        dt.update(&bids, &asks);
        let bids2 = vec![(100, 75)];
        let asks2 = vec![(101, 75)];
        dt.update(&bids2, &asks2);
        assert_eq!(dt.score(), 50);
    }

    #[test]
    fn depth_increasing_score_zero() {
        let mut dt = DepthThinning::new(10);
        let bids = vec![(100, 50)];
        let asks = vec![(101, 50)];
        dt.update(&bids, &asks);
        let bids2 = vec![(100, 100)];
        let asks2 = vec![(101, 100)];
        dt.update(&bids2, &asks2);
        assert_eq!(dt.score(), 0);
    }

    #[test]
    fn only_top_n_levels_counted() {
        let mut dt = DepthThinning::new(2);
        let bids = vec![(100, 50), (99, 30), (98, 9999)];
        let asks = vec![(101, 50), (102, 30), (103, 9999)];
        dt.update(&bids, &asks);
        let bids2 = vec![(100, 25), (99, 15), (98, 9999)];
        let asks2 = vec![(101, 25), (102, 15), (103, 9999)];
        dt.update(&bids2, &asks2);
        assert_eq!(dt.score(), 100);
    }
}
