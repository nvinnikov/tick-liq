//! Property-based tests for `tick_liq::math::il`.
//!
//! Invariants (256+ cases each):
//!   1. IL ≤ 0 for all valid inputs (the LP never outperforms HODL).
//!   2. IL == 0 exactly when current_price == entry_price (within a tiny
//!      float tolerance).
//!   3. Asymptotic bound: as price moves far outside the range, IL remains
//!      strictly > -1 (position is never literally worthless) and
//!      monotone non-increasing as the price moves further away.

use proptest::prelude::*;
use tick_liq::math::il::impermanent_loss;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn il_is_non_positive(
        entry in 10f64..10_000.0,
        current in 1f64..100_000.0,
        lower_frac in 0.1f64..0.95,
        upper_frac in 1.05f64..10.0,
    ) {
        let lower = entry * lower_frac;
        let upper = entry * upper_frac;
        let il = impermanent_loss(entry, current, lower, upper).unwrap();
        prop_assert!(il <= 1e-12, "IL should be ≤ 0, got {}", il);
    }

    #[test]
    fn il_zero_at_identity(
        entry in 10f64..10_000.0,
        lower_frac in 0.1f64..0.95,
        upper_frac in 1.05f64..10.0,
    ) {
        let lower = entry * lower_frac;
        let upper = entry * upper_frac;
        let il = impermanent_loss(entry, entry, lower, upper).unwrap();
        prop_assert!(il.abs() < 1e-12, "IL at identity should be 0, got {}", il);
    }

    #[test]
    fn il_bounded_above_minus_one(
        entry in 10f64..10_000.0,
        current_far in 0.0001f64..1_000_000.0,
        lower_frac in 0.5f64..0.99,
        upper_frac in 1.01f64..2.0,
    ) {
        let lower = entry * lower_frac;
        let upper = entry * upper_frac;
        let il = impermanent_loss(entry, current_far, lower, upper).unwrap();
        prop_assert!(il > -1.0, "IL must be > -1, got {}", il);
        prop_assert!(il <= 1e-12);
    }

    #[test]
    fn il_monotone_moving_further_below(
        entry in 100f64..1_000.0,
        lower_frac in 0.5f64..0.95,
        upper_frac in 1.05f64..2.0,
        p1 in 0.01f64..0.49,
        p2_extra in 0.0001f64..0.48,
    ) {
        let lower = entry * lower_frac;
        let upper = entry * upper_frac;
        let current_1 = entry * p1;
        let current_2 = entry * (p1 - p2_extra).max(1e-6);
        prop_assume!(current_2 < current_1);

        let il_1 = impermanent_loss(entry, current_1, lower, upper).unwrap();
        let il_2 = impermanent_loss(entry, current_2, lower, upper).unwrap();
        // Moving further below the range cannot *improve* IL — it should be
        // at least as negative. Allow a tiny float slack.
        prop_assert!(il_2 <= il_1 + 1e-12,
            "IL non-monotone as price moves further below: {} -> {}", il_1, il_2);
    }
}
