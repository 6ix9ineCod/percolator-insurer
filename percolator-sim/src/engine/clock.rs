pub struct SlotClock {
    current: u64,
    start_timestamp_ms: u64,
    slot_duration_ms: u64,
    started: bool,
}

impl SlotClock {
    pub fn new(init_slot: u64, slot_duration_ms: u64) -> Self {
        Self {
            current: init_slot,
            start_timestamp_ms: 0,
            slot_duration_ms,
            started: false,
        }
    }

    pub fn set_start(&mut self, timestamp_ms: u64) {
        self.start_timestamp_ms = timestamp_ms;
        self.started = true;
    }

    pub fn start_timestamp_ms(&self) -> u64 {
        self.start_timestamp_ms
    }

    pub fn current_slot(&self) -> u64 {
        self.current
    }

    pub fn slot_duration_ms(&self) -> u64 {
        self.slot_duration_ms
    }

    pub fn advance_to(&mut self, timestamp_ms: u64) -> u64 {
        if !self.started {
            self.set_start(timestamp_ms);
            return 0;
        }
        if timestamp_ms <= self.start_timestamp_ms {
            return 0;
        }
        let elapsed_ms = timestamp_ms - self.start_timestamp_ms;
        let target_slot = elapsed_ms / self.slot_duration_ms;
        if target_slot <= self.current {
            return 0;
        }
        let advanced = target_slot - self.current;
        self.current = target_slot;
        advanced
    }

    pub fn slot_to_ms(&self, slot: u64) -> u64 {
        slot * self.slot_duration_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_clock_starts_at_init_slot() {
        let clock = SlotClock::new(1000, 400);
        assert_eq!(clock.current_slot(), 1000);
        assert_eq!(clock.start_timestamp_ms(), 0);
    }

    #[test]
    fn advance_to_same_timestamp_no_change() {
        let mut clock = SlotClock::new(0, 400);
        clock.set_start(5000);
        let advanced = clock.advance_to(5000);
        assert_eq!(advanced, 0);
        assert_eq!(clock.current_slot(), 0);
    }

    #[test]
    fn advance_by_one_slot() {
        let mut clock = SlotClock::new(0, 400);
        clock.set_start(5000);
        let advanced = clock.advance_to(5400);
        assert_eq!(advanced, 1);
        assert_eq!(clock.current_slot(), 1);
    }

    #[test]
    fn advance_by_many_slots() {
        let mut clock = SlotClock::new(0, 400);
        clock.set_start(0);
        let advanced = clock.advance_to(2000);
        assert_eq!(advanced, 5);
        assert_eq!(clock.current_slot(), 5);
    }

    #[test]
    fn advance_partial_slot_no_advance() {
        let mut clock = SlotClock::new(0, 400);
        clock.set_start(0);
        let advanced = clock.advance_to(399);
        assert_eq!(advanced, 0);
        assert_eq!(clock.current_slot(), 0);
    }

    #[test]
    fn advance_is_monotonic() {
        let mut clock = SlotClock::new(10, 400);
        clock.set_start(1000);
        clock.advance_to(5000); // (5000-1000)/400 = 10, which is >= init(10), so current=10
        let advanced = clock.advance_to(5800); // (5800-1000)/400 = 12
        assert_eq!(advanced, 2);
        assert_eq!(clock.current_slot(), 12);
        let regress = clock.advance_to(5400); // earlier → no change
        assert_eq!(regress, 0);
        assert_eq!(clock.current_slot(), 12);
    }

    #[test]
    fn slot_to_timestamp() {
        let clock = SlotClock::new(0, 400);
        assert_eq!(clock.slot_to_ms(0), 0);
        assert_eq!(clock.slot_to_ms(5), 2000);
        assert_eq!(clock.slot_to_ms(100), 40000);
    }
}
