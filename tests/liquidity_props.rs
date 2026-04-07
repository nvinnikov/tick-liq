//! Property-based tests for `tick_liq::math::liquidity`.
//!
//! Invariants enforced (256+ cases each):
//!   1. amounts non-negative (trivially u64, but we also assert at-least-one
//!      side is zero outside the range)
//!   2. regime correctness:
//!        - P < P_a → amount_y == 0
//!        - P ≥ P_b → amount_x == 0
//!   3. round-trip: liquidity → amounts → liquidity returns L' ≤ L (floor on
//!      every step) and the relative error is below 1e-5 for non-tiny L
//!   4. monotonicity in liquidity: doubling L cannot decrease either amount

use orca_whirlpools_core::tick_index_to_sqrt_price;
use proptest::prelude::*;
use tick_liq::math::liquidity::{get_amounts_for_liquidity, get_liquidity_for_amounts};

fn sp(tick: i32) -> u128 {
    tick_index_to_sqrt_price(tick)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn regime_below_range_no_y(
        liquidity in 1u128..1_000_000_000_000u128,
        gap in 1i32..10_000,
        width in 10i32..50_000,
    ) {
        let lower = gap;
        let upper = lower + width;
        let current = 0; // strictly below `lower`
        let amts = get_amounts_for_liquidity(liquidity, sp(current), sp(lower), sp(upper)).unwrap();
        prop_assert_eq!(amts.amount_y, 0);
    }

    #[test]
    fn regime_above_range_no_x(
        liquidity in 1u128..1_000_000_000_000u128,
        lower in -10_000i32..0,
        width in 10i32..1_000,
    ) {
        let upper = lower + width;
        let current = upper + 100; // strictly above `upper`
        let amts = get_amounts_for_liquidity(liquidity, sp(current), sp(lower), sp(upper)).unwrap();
        prop_assert_eq!(amts.amount_x, 0);
    }

    #[test]
    fn round_trip_in_range_floor_bound(
        liquidity in 1_000_000_000u128..1_000_000_000_000_000u128,
        lower in -10_000i32..0,
        offset_to_current in 1i32..500,
        width in 100i32..2_000,
    ) {
        let upper = lower + width;
        prop_assume!(offset_to_current < width);
        let current = lower + offset_to_current;

        let amts = get_amounts_for_liquidity(liquidity, sp(current), sp(lower), sp(upper)).unwrap();
        let recovered = get_liquidity_for_amounts(
            sp(current), sp(lower), sp(upper), amts.amount_x, amts.amount_y,
        ).unwrap();

        // Floor everywhere ⇒ recovered ≤ liquidity.
        prop_assert!(recovered <= liquidity);
        // Relative error below 1e-5 for non-tiny L (we filtered to L ≥ 1e9).
        let drift = liquidity - recovered;
        prop_assert!(drift <= liquidity / 100_000);
    }

    #[test]
    fn monotone_in_liquidity(
        l in 1u128..1_000_000_000u128,
        lower in -5_000i32..0,
        offset in 1i32..200,
        width in 50i32..1_000,
    ) {
        let upper = lower + width;
        prop_assume!(offset < width);
        let current = lower + offset;

        let a1 = get_amounts_for_liquidity(l, sp(current), sp(lower), sp(upper)).unwrap();
        let a2 = get_amounts_for_liquidity(l * 2, sp(current), sp(lower), sp(upper)).unwrap();
        prop_assert!(a2.amount_x >= a1.amount_x);
        prop_assert!(a2.amount_y >= a1.amount_y);
    }
}
