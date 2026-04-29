use percolator::MAX_ACCOUNTS;

pub struct AccountManager {
    trade_count: u16,
    lp_count: u16,
    free_queue: Vec<u16>,
    positioned: Vec<bool>,
    next_robin: usize,
}

impl AccountManager {
    pub fn new(trade_accounts: u16, lp_accounts: u16) -> Self {
        assert!((trade_accounts + lp_accounts) as usize <= MAX_ACCOUNTS);
        let mut free_queue = Vec::with_capacity(trade_accounts as usize);
        for i in 0..trade_accounts {
            free_queue.push(i);
        }
        Self {
            trade_count: trade_accounts,
            lp_count: lp_accounts,
            free_queue,
            positioned: vec![false; MAX_ACCOUNTS],
            next_robin: 0,
        }
    }

    pub fn next_trade_account(&self) -> Option<u16> {
        self.free_queue.first().copied()
    }

    pub fn allocate_trade_account(&mut self) -> Option<u16> {
        if self.free_queue.is_empty() {
            return None;
        }
        let idx = self.next_robin % self.free_queue.len();
        let account = self.free_queue.remove(idx);
        self.next_robin = if self.free_queue.is_empty() { 0 } else { idx % self.free_queue.len() };
        Some(account)
    }

    pub fn release_trade_account(&mut self, idx: u16) {
        if idx < self.trade_count && !self.free_queue.contains(&idx) {
            self.free_queue.push(idx);
        }
    }

    pub fn lp_accounts(&self) -> Vec<u16> {
        (self.trade_count..self.trade_count + self.lp_count).collect()
    }

    pub fn mark_positioned(&mut self, idx: u16) {
        if (idx as usize) < self.positioned.len() {
            self.positioned[idx as usize] = true;
        }
    }

    pub fn mark_flat(&mut self, idx: u16) {
        if (idx as usize) < self.positioned.len() {
            self.positioned[idx as usize] = false;
        }
    }

    pub fn is_positioned(&self, idx: u16) -> bool {
        self.positioned.get(idx as usize).copied().unwrap_or(false)
    }

    pub fn positioned_accounts(&self) -> Vec<u16> {
        (0..self.trade_count)
            .filter(|&i| self.positioned[i as usize])
            .collect()
    }

    pub fn free_count(&self) -> usize {
        self.free_queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_manager_all_free() {
        let am = AccountManager::new(60, 4);
        assert_eq!(am.next_trade_account(), Some(0));
    }

    #[test]
    fn allocate_round_robin() {
        let mut am = AccountManager::new(60, 4);
        assert_eq!(am.allocate_trade_account(), Some(0));
        assert_eq!(am.allocate_trade_account(), Some(1));
        assert_eq!(am.allocate_trade_account(), Some(2));
    }

    #[test]
    fn release_makes_available() {
        let mut am = AccountManager::new(2, 2);
        assert_eq!(am.allocate_trade_account(), Some(0));
        assert_eq!(am.allocate_trade_account(), Some(1));
        assert_eq!(am.allocate_trade_account(), None);
        am.release_trade_account(0);
        assert_eq!(am.allocate_trade_account(), Some(0));
    }

    #[test]
    fn lp_accounts_separate() {
        let am = AccountManager::new(60, 4);
        let lps = am.lp_accounts();
        assert_eq!(lps, vec![60, 61, 62, 63]);
    }

    #[test]
    fn active_positions_tracked() {
        let mut am = AccountManager::new(60, 4);
        am.allocate_trade_account();
        am.mark_positioned(0);
        assert_eq!(am.positioned_accounts(), vec![0]);
        am.mark_flat(0);
        assert!(am.positioned_accounts().is_empty());
    }
}
