//! Property-based tests for the analytics math functions.
//!
//! Task F2: ensures invariants hold across the full input space using
//! proptest with at least 256 cases per property.
//!
//! Because the crate is currently a binary-only crate (no `lib.rs`),
//! we pull the analytics source files into this integration test via
//! `#[path]` module declarations. This keeps the change test-only.

#![allow(dead_code)]

#[path = "../src/analytics/amounts.rs"]
mod amounts;

#[path = "../src/math/il.rs"]
mod pnl;

#[path = "../src/math/greeks.rs"]
mod greeks_math;

#[path = "../src/math/sqrt_price.rs"]
mod sqrt_price;

#[path = "../src/math/impact.rs"]
mod depth;

use orca_whirlpools_core::tick_index_to_sqrt_price;
use proptest::prelude::*;

use amounts::compute_token_amounts;
use depth::estimate_impact;
use greeks_math::compute_greeks_from_prices;
use pnl::compute_il;
use sqrt_price::sqrt_q64_to_price;

// Orca tick range constants. The full SDK range is roughly [-443636, 443636].
// We use a tighter range to keep token-amount math from overflowing u64
// (Orca's amount-delta math returns an error if the result exceeds u64::MAX).
// A ±100k tick band combined with the liquidity strategy below keeps every
// computed amount comfortably within u64.
const MIN_TICK: i32 = -100_000;
const MAX_TICK: i32 = 100_000;

const EPS: f64 = 1e-9;

/// Strategy producing an ordered (lower < upper) tick pair within the valid range.
fn tick_pair() -> impl Strategy<Value = (i32, i32)> {
    (MIN_TICK..=MAX_TICK, MIN_TICK..=MAX_TICK).prop_filter_map("lower < upper", |(a, b)| {
        if a == b {
            None
        } else if a < b {
            Some((a, b))
        } else {
            Some((b, a))
        }
    })
}

/// A liquidity range that exercises both small and large positions but
/// stays well below `u128::MAX` to avoid math overflow.
fn liquidity() -> impl Strategy<Value = u128> {
    // Cap below 2^40 ≈ 1e12. Combined with the ±100k tick band this keeps
    // `compute_token_amounts` results inside u64::MAX for the worst-case
    // wide-range, far-from-center inputs Orca's math will accept.
    1u128..=10u128.pow(12)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 1: token amounts are always non-negative for any valid input.
    /// (u64 is unsigned, so we additionally assert that the call succeeds.)
    #[test]
    fn token_amounts_non_negative(
        liq in liquidity(),
        (tl, tu) in tick_pair(),
        tc in MIN_TICK..=MAX_TICK,
    ) {
        let sqrt_price = tick_index_to_sqrt_price(tc);
        let res = compute_token_amounts(liq, sqrt_price, tl, tu);
        prop_assert!(res.is_ok(), "compute_token_amounts failed: {:?}", res.err());
        let amounts = res.unwrap();
        // `amount_a` / `amount_b` are `u64`, so non-negativity is guaranteed
        // by the type. The real invariant we want to enforce is that the
        // computation succeeds (does not overflow u64) for any valid input
        // within the strategy domain.
        let _ = (amounts.amount_a, amounts.amount_b);
    }

    /// Property 2: when current tick is strictly below the range, the
    /// position is 100% token A. (For non-zero liquidity, amount_a > 0.)
    #[test]
    fn below_range_only_token_a(
        liq in liquidity(),
        (tl, tu) in tick_pair(),
        offset in 1i32..=10_000,
    ) {
        // Skip if shifting below would underflow the tick range.
        prop_assume!(tl.saturating_sub(offset) >= MIN_TICK);
        let tc = tl - offset;
        let sqrt_price = tick_index_to_sqrt_price(tc);
        let amounts = compute_token_amounts(liq, sqrt_price, tl, tu).unwrap();
        prop_assert_eq!(amounts.amount_b, 0, "amount_b must be 0 below range");
        prop_assert!(amounts.amount_a > 0, "amount_a must be > 0 below range with liquidity > 0");
    }

    /// Property 3: when current tick is strictly above the range, the
    /// position is 100% token B.
    #[test]
    fn above_range_only_token_b(
        liq in liquidity(),
        (tl, tu) in tick_pair(),
        offset in 1i32..=10_000,
    ) {
        prop_assume!(tu.saturating_add(offset) <= MAX_TICK);
        let tc = tu + offset;
        let sqrt_price = tick_index_to_sqrt_price(tc);
        let amounts = compute_token_amounts(liq, sqrt_price, tl, tu).unwrap();
        prop_assert_eq!(amounts.amount_a, 0, "amount_a must be 0 above range");
        prop_assert!(amounts.amount_b > 0, "amount_b must be > 0 above range with liquidity > 0");
    }

    /// Property 4: impermanent loss is always non-positive (within epsilon)
    /// for any valid positive prices.
    #[test]
    fn il_non_positive(
        price_entry in 1e-6f64..1e9,
        price_current in 1e-6f64..1e9,
        plo in 1e-6f64..1e9,
        phi_mul in 1.0001f64..1e6,
    ) {
        let phi = plo * phi_mul; // ensures phi > plo
        let il = compute_il(price_entry, price_current, plo, phi);
        prop_assert!(il <= EPS, "IL must be <= 0, got {}", il);
        prop_assert!(il.is_finite(), "IL must be finite");
    }

    /// Property 5: IL at the entry price equals zero (within epsilon).
    #[test]
    fn il_zero_at_identity(
        p in 1e-6f64..1e9,
        plo in 1e-6f64..1e9,
        phi_mul in 1.0001f64..1e6,
    ) {
        let phi = plo * phi_mul;
        let il = compute_il(p, p, plo, phi);
        prop_assert!(il.abs() < 1e-12, "IL at identity must be ~0, got {}", il);
    }

    /// Property 6: when the price is strictly inside the range, the LP
    /// delta is non-positive (LP is naturally short volatility).
    #[test]
    fn greeks_delta_non_positive_in_range(
        liq in liquidity(),
        (tl, tu) in tick_pair(),
    ) {
        // Pick a tick strictly inside [tl, tu]. We need at least 2 ticks of room.
        prop_assume!(tu - tl >= 2);
        let tc = tl + (tu - tl) / 2; // midpoint, guaranteed strictly inside
        prop_assume!(tc > tl && tc < tu);
        let price = sqrt_q64_to_price(tick_index_to_sqrt_price(tc));
        let price_lower = sqrt_q64_to_price(tick_index_to_sqrt_price(tl));
        let price_upper = sqrt_q64_to_price(tick_index_to_sqrt_price(tu));
        let g = compute_greeks_from_prices(liq, price, price_lower, price_upper);
        prop_assert!(g.delta <= EPS, "delta must be <= 0 in range, got {}", g.delta);
    }

    /// Property 7: price impact is monotonically increasing in trade size.
    /// Doubling the target percentage should not decrease the USD needed.
    #[test]
    fn impact_monotone_in_size(
        price in 0.01f64..1e6,
        liq in 1u128..=10u128.pow(15),
        pct in 0.01f64..10.0,
        is_buy in any::<bool>(),
    ) {
        let small = estimate_impact(price, liq, pct, is_buy);
        let large = estimate_impact(price, liq, pct * 2.0, is_buy);
        prop_assert!(
            large.usd_needed + EPS >= small.usd_needed,
            "doubling size should not decrease impact: small={}, large={}",
            small.usd_needed, large.usd_needed
        );
    }

    /// Property 8: more pool liquidity → less impact for the same target
    /// percentage. Equivalently, doubling liquidity doubles the USD needed,
    /// so it must be non-decreasing.
    #[test]
    fn impact_monotone_in_liquidity(
        price in 0.01f64..1e6,
        liq in 1u128..=10u128.pow(14),
        pct in 0.01f64..10.0,
        is_buy in any::<bool>(),
    ) {
        let thin = estimate_impact(price, liq, pct, is_buy);
        let deep = estimate_impact(price, liq.saturating_mul(10), pct, is_buy);
        prop_assert!(
            deep.usd_needed + EPS >= thin.usd_needed,
            "more liquidity should require >= USD for same %: thin={}, deep={}",
            thin.usd_needed, deep.usd_needed
        );
    }
}
