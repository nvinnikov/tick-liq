//! Property-based tests for `tick_liq::math::greeks`.
//!
//! Invariants (256+ cases each):
//!   1. delta ≤ 0 in-range (LP is short the base asset)
//!   2. gamma ≥ 0 in-range (LP is short volatility / concave)
//!   3. delta == 0 and gamma == 0 out-of-range
//!   4. delta and gamma both scale linearly in liquidity L

use proptest::prelude::*;
use tick_liq::math::greeks::{compute_greeks, Greeks};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn delta_non_positive_gamma_non_negative_in_range(
        l in 1u128..1_000_000_000_000u128,
        mid in 10f64..10_000.0,
        lower_frac in 0.1f64..0.99,
        upper_frac in 1.01f64..10.0,
    ) {
        let lower = mid * lower_frac;
        let upper = mid * upper_frac;
        let g = compute_greeks(l, mid, lower, upper).unwrap();
        prop_assert!(g.delta <= 0.0);
        prop_assert!(g.gamma >= 0.0);
    }

    #[test]
    fn zero_out_of_range(
        l in 1u128..1_000_000_000_000u128,
        lower in 10f64..1_000.0,
        width_frac in 0.01f64..0.5,
        side in any::<bool>(),
    ) {
        let upper = lower * (1.0 + width_frac);
        let price = if side {
            lower * 0.5 // below range
        } else {
            upper * 2.0 // above range
        };
        let g = compute_greeks(l, price, lower, upper).unwrap();
        prop_assert_eq!(g, Greeks::ZERO);
    }

    #[test]
    fn linear_in_liquidity(
        l in 1u128..1_000_000_000u128,
        mid in 10f64..1_000.0,
        lower_frac in 0.2f64..0.95,
        upper_frac in 1.05f64..5.0,
        scale in 2u64..100,
    ) {
        let lower = mid * lower_frac;
        let upper = mid * upper_frac;
        let g1 = compute_greeks(l, mid, lower, upper).unwrap();
        let g2 = compute_greeks(l * scale as u128, mid, lower, upper).unwrap();
        // Linearity: g2 = scale * g1 (allow tiny relative float drift).
        let target_delta = g1.delta * scale as f64;
        let target_gamma = g1.gamma * scale as f64;
        let drift_d = ((g2.delta - target_delta) / target_delta).abs();
        let drift_g = ((g2.gamma - target_gamma) / target_gamma).abs();
        prop_assert!(drift_d < 1e-9, "delta drift {}", drift_d);
        prop_assert!(drift_g < 1e-9, "gamma drift {}", drift_g);
    }
}
