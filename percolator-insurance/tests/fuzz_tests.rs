use proptest::prelude::*;
use percolator_insurance::premium::{compute_premium_per_slot, isqrt, leverage_multiplier};
use percolator_insurance::pool::PremiumPool;
use percolator_insurance::risk_index::RiskIndex;
use percolator_insurance::MULT_SCALE;

proptest! {
    #[test]
    fn fuzz_isqrt_correct(n in 0u128..=u64::MAX as u128) {
        let r = isqrt(n);
        prop_assert!(r * r <= n, "isqrt({})={}, {}^2={} > {}", n, r, r, r * r, n);
        if r < u64::MAX as u128 {
            let next = r + 1;
            prop_assert!(next * next > n, "isqrt({})={}, ({}+1)^2={} <= {}", n, r, r, next * next, n);
        }
    }

    #[test]
    fn fuzz_premium_monotonic_notional(
        notional1 in 1u128..1_000_000_000u128,
        notional2 in 1u128..1_000_000_000u128,
        capital in 1u128..1_000_000_000u128,
    ) {
        let idx = RiskIndex::neutral();
        let p1 = compute_premium_per_slot(notional1, capital, 100, &idx, 1);
        let p2 = compute_premium_per_slot(notional2, capital, 100, &idx, 1);

        if notional1 <= notional2 {
            prop_assert!(p1 <= p2, "premium must increase with notional: {}@{} vs {}@{}", p1, notional1, p2, notional2);
        }
    }

    #[test]
    fn fuzz_premium_monotonic_leverage(
        notional in 10_000u128..1_000_000u128,
        capital1 in 1u128..1_000_000u128,
        capital2 in 1u128..1_000_000u128,
    ) {
        let idx = RiskIndex::neutral();
        let p1 = compute_premium_per_slot(notional, capital1, 100, &idx, 1);
        let p2 = compute_premium_per_slot(notional, capital2, 100, &idx, 1);

        if capital1 >= capital2 {
            prop_assert!(p1 <= p2, "premium must decrease with higher capital: cap1={} p1={}, cap2={} p2={}", capital1, p1, capital2, p2);
        }
    }

    #[test]
    fn fuzz_pool_invariants(
        ops in prop::collection::vec(
            prop::bool::ANY.prop_flat_map(|is_collect| {
                if is_collect {
                    (Just(true), 1u128..1_000_000u128).boxed()
                } else {
                    (Just(false), 1u128..1_000_000u128).boxed()
                }
            }),
            1..50
        )
    ) {
        let mut pool = PremiumPool::new();
        for (is_collect, amount) in ops {
            if is_collect {
                let _ = pool.record_collection(amount);
            } else {
                pool.record_consumption(amount);
            }
            prop_assert!(pool.check_invariants(), "invariant violated: {:?}", pool);
        }
    }

    #[test]
    fn fuzz_leverage_mult_floor(
        notional in 0u128..1_000_000_000u128,
        capital in 1u128..1_000_000_000u128,
        exp_num in 1u64..5u64,
        exp_den in 1u64..3u64,
    ) {
        let (num, den) = leverage_multiplier(notional, capital, exp_num, exp_den);
        prop_assert!(den > 0, "denominator must be positive");
        prop_assert!(num >= MULT_SCALE, "multiplier num must be >= MULT_SCALE: got {}", num);
    }

    #[test]
    fn fuzz_premium_never_panics(
        notional in 0u128..u64::MAX as u128,
        capital in 0u128..u64::MAX as u128,
        base_rate in 0u128..1_000_000u128,
        crowd_num in 1u128..10_000u128,
        oiv_num in 1u128..10_000u128,
        pool_num in 1u128..10_000u128,
        vol_num in 1u128..10_000u128,
        tail_num in 1u128..10_000u128,
    ) {
        let idx = RiskIndex {
            crowding: (crowd_num, MULT_SCALE),
            oi_vault: (oiv_num, MULT_SCALE),
            pool_health: (pool_num, MULT_SCALE),
            volatility: (vol_num, MULT_SCALE),
            leverage_tail: (tail_num, MULT_SCALE),
        };
        let _ = compute_premium_per_slot(notional, capital, base_rate, &idx, 1);
    }
}
