use percolator_insurance::pool::PremiumPool;

// ============================================================================
// new
// ============================================================================

#[test]
fn test_pool_new() {
    let pool = PremiumPool::new();
    assert_eq!(pool.balance, 0);
    assert_eq!(pool.total_collected, 0);
    assert_eq!(pool.total_paid_out, 0);
    assert_eq!(pool.last_deficit_check_slot, 0);
    assert!(pool.check_invariants());
}

// ============================================================================
// record_collection
// ============================================================================

#[test]
fn test_pool_record_collection() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    assert_eq!(pool.balance, 1000);
    assert_eq!(pool.total_collected, 1000);
    assert_eq!(pool.total_paid_out, 0);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_record_multiple_collections() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    pool.record_collection(500).unwrap();
    assert_eq!(pool.balance, 1500);
    assert_eq!(pool.total_collected, 1500);
    assert_eq!(pool.total_paid_out, 0);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_record_collection_overflow() {
    let mut pool = PremiumPool::new();
    // Saturate balance and total_collected near max
    pool.record_collection(u128::MAX / 2).unwrap();
    // This second collection should overflow total_collected
    let result = pool.record_collection(u128::MAX / 2 + 2);
    assert!(result.is_err());
}

// ============================================================================
// record_consumption
// ============================================================================

#[test]
fn test_pool_record_consumption() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    pool.record_consumption(300);
    assert_eq!(pool.balance, 700);
    assert_eq!(pool.total_paid_out, 300);
    assert_eq!(pool.total_collected, 1000);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_consumption_capped_at_balance() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    pool.record_consumption(2000);
    assert_eq!(pool.balance, 0);
    assert_eq!(pool.total_paid_out, 1000);
    assert_eq!(pool.total_collected, 1000);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_consumption_zero() {
    let mut pool = PremiumPool::new();
    pool.record_collection(500).unwrap();
    pool.record_consumption(0);
    assert_eq!(pool.balance, 500);
    assert_eq!(pool.total_paid_out, 0);
    assert!(pool.check_invariants());
}

// ============================================================================
// reconcile_with_insurance_balance
// ============================================================================

#[test]
fn test_pool_reconcile_deficit() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    let consumed = pool.reconcile_with_insurance_balance(400);
    assert_eq!(consumed, 600);
    assert_eq!(pool.balance, 400);
    assert_eq!(pool.total_paid_out, 600);
    assert_eq!(pool.total_collected, 1000);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_reconcile_no_deficit() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    let consumed = pool.reconcile_with_insurance_balance(2000);
    assert_eq!(consumed, 0);
    assert_eq!(pool.balance, 1000);
    assert_eq!(pool.total_paid_out, 0);
    assert!(pool.check_invariants());
}

#[test]
fn test_pool_reconcile_exact_balance() {
    let mut pool = PremiumPool::new();
    pool.record_collection(1000).unwrap();
    let consumed = pool.reconcile_with_insurance_balance(1000);
    assert_eq!(consumed, 0);
    assert_eq!(pool.balance, 1000);
    assert_eq!(pool.total_paid_out, 0);
    assert!(pool.check_invariants());
}

// ============================================================================
// invariant conservation
// ============================================================================

#[test]
fn test_pool_invariant_conservation() {
    let mut pool = PremiumPool::new();
    pool.record_collection(5000).unwrap();
    assert!(pool.check_invariants());
    pool.record_consumption(1200);
    assert!(pool.check_invariants());
    pool.record_collection(300).unwrap();
    assert!(pool.check_invariants());
    pool.record_consumption(800);
    assert!(pool.check_invariants());
    // balance = 5000 - 1200 + 300 - 800 = 3300
    // total_collected = 5000 + 300 = 5300
    // total_paid_out = 1200 + 800 = 2000
    // balance + paid_out = 3300 + 2000 = 5300 == total_collected
    assert_eq!(pool.balance, 3300);
    assert_eq!(pool.total_paid_out, 2000);
    assert_eq!(pool.total_collected, 5300);
    assert_eq!(pool.balance + pool.total_paid_out, pool.total_collected);
}

#[test]
fn test_pool_invariant_after_reconcile() {
    let mut pool = PremiumPool::new();
    pool.record_collection(3000).unwrap();
    pool.record_collection(2000).unwrap();
    let consumed = pool.reconcile_with_insurance_balance(1500);
    assert_eq!(consumed, 3500);
    assert!(pool.check_invariants());
}
