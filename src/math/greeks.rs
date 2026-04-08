//! LP Greeks (delta, gamma) for ranged concentrated-liquidity positions.
//!
//! Per `CLAUDE.md` (Math Reference):
//!
//! ```text
//! delta = dx/dP  = -L / (2 · √P · P)      when P ∈ [Pa, Pb]
//! gamma = d²x/dP² = L / (2 · P² · √P)     when P ∈ [Pa, Pb]
//! ```
//!
//! Both are **zero when the price is outside the range**: the position is
//! 100 % in one asset and the inventory does not change with price until
//! the price re-enters the range.
//!
//! ## Semantics — these are *inventory* Greeks, not *value* Greeks
//!
//! `delta` here is `∂(amount of token x held)/∂P`, not `∂(position value
//! in quote)/∂P`. It answers "how much base does the LP *sell* per unit
//! price move?" — hence always non-positive in-range, because an LP auto-
//! sells base into a rising market. This is the quantity the delta-hedge
//! engine needs: it tells you how many units of the perp to short to
//! neutralize the position's base-asset exposure. It is **not** the same
//! as `dV/dP`, which would be positive and much larger in magnitude.
//!
//! `gamma` here is the second derivative of the same inventory: how
//! quickly `delta` itself changes per unit price move. It is positive
//! in-range and governs how frequently the hedge has to be adjusted.
//!
//! Both quantities scale linearly with liquidity `L`.
//!
//! ## Why this lives in `math` (and not `analytics`)
//!
//! The existing `src/analytics/greeks.rs` has a working `u128`-sqrt-price
//! implementation used by the LP Inspector. This module is the canonical
//! version the strategy/execution layers will use and accepts display-unit
//! prices for consistency with [`crate::math::il`]. The analytics copy will
//! be migrated in a follow-up once the hedge engine consumes this API.

use anyhow::{bail, Result};

/// LP position Greeks at a given price.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Greeks {
    /// `∂x / ∂P` — rate of change of the *amount of token x held* per unit
    /// price increase (inventory delta, not value delta). Non-positive in
    /// range (LP auto-sells base into a rising market). Zero out of range.
    pub delta: f64,
    /// `∂²x / ∂P²` — second derivative of inventory. Non-negative in range.
    /// Zero out of range.
    pub gamma: f64,
}

impl Greeks {
    pub const ZERO: Self = Greeks {
        delta: 0.0,
        gamma: 0.0,
    };
}

/// Compute the LP Greeks for a position with liquidity `L` at the current
/// `price`, bounded by `[lower_price, upper_price]`. All prices are in
/// display units (quote per base), i.e. the same convention as
/// [`crate::math::il::impermanent_loss`].
///
/// Returns [`Greeks::ZERO`] whenever the price is outside the range.
///
/// # Errors
/// Returns an error if any price is non-positive or non-finite, or if
/// `lower_price ≥ upper_price`. Liquidity is accepted as `u128` (on-chain
/// native) and cast to `f64` internally; for `L > 2^53` some lower bits are
/// dropped, which is acceptable because the Greeks are consumed as
/// risk-control inputs, not as exact accounting figures.
pub fn compute_greeks(
    liquidity: u128,
    price: f64,
    lower_price: f64,
    upper_price: f64,
) -> Result<Greeks> {
    for (name, v) in [
        ("price", price),
        ("lower_price", lower_price),
        ("upper_price", upper_price),
    ] {
        if !v.is_finite() || v <= 0.0 {
            bail!("{} must be finite and positive, got {}", name, v);
        }
    }
    if lower_price >= upper_price {
        bail!("lower_price must be < upper_price");
    }

    if price < lower_price || price > upper_price {
        return Ok(Greeks::ZERO);
    }

    let l = liquidity as f64;
    let sqrt_p = price.sqrt();

    // delta = -L / (2 · sqrt(P) · P)
    let delta = -l / (2.0 * sqrt_p * price);
    // gamma = L / (2 · P^(5/2))
    let gamma = l / (2.0 * price * price * sqrt_p);

    Ok(Greeks { delta, gamma })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_negative_and_gamma_positive_in_range() {
        let g = compute_greeks(1_000_000, 100.0, 80.0, 125.0).unwrap();
        assert!(g.delta < 0.0);
        assert!(g.gamma > 0.0);
    }

    #[test]
    fn zero_out_of_range_below() {
        let g = compute_greeks(1_000_000, 50.0, 80.0, 125.0).unwrap();
        assert_eq!(g, Greeks::ZERO);
    }

    #[test]
    fn zero_out_of_range_above() {
        let g = compute_greeks(1_000_000, 200.0, 80.0, 125.0).unwrap();
        assert_eq!(g, Greeks::ZERO);
    }

    #[test]
    fn delta_scales_linearly_with_liquidity() {
        let g1 = compute_greeks(1_000, 100.0, 80.0, 125.0).unwrap();
        let g10 = compute_greeks(10_000, 100.0, 80.0, 125.0).unwrap();
        let ratio = g10.delta / g1.delta;
        assert!((ratio - 10.0).abs() < 1e-9, "ratio={}", ratio);
    }

    #[test]
    fn invalid_inputs_error() {
        assert!(compute_greeks(1_000, 0.0, 80.0, 125.0).is_err());
        assert!(compute_greeks(1_000, 100.0, 125.0, 80.0).is_err());
        assert!(compute_greeks(1_000, f64::NAN, 80.0, 125.0).is_err());
        assert!(compute_greeks(1_000, f64::INFINITY, 80.0, 125.0).is_err());
    }

    #[test]
    fn delta_matches_finite_difference_of_inventory() {
        // The analytical formula is delta = dx/dP where x = L*(1/√P - 1/√Pb).
        // Verify it numerically with a centered finite difference.
        let p = 100.0;
        let lower = 80.0;
        let upper = 125.0;
        let l: u128 = 1_000_000_000;
        let g = compute_greeks(l, p, lower, upper).unwrap();

        let h = 0.001;
        let x_plus = inventory_x(l, p + h, lower, upper);
        let x_minus = inventory_x(l, p - h, lower, upper);
        let delta_fd = (x_plus - x_minus) / (2.0 * h);

        let rel_err = ((g.delta - delta_fd) / g.delta).abs();
        assert!(
            rel_err < 1e-6,
            "analytic delta={}, fd={}, rel_err={}",
            g.delta,
            delta_fd,
            rel_err
        );
    }

    // Helper: amount of token x in the position at price `p`, in raw units.
    fn inventory_x(liquidity: u128, p: f64, lower: f64, upper: f64) -> f64 {
        let sa = lower.sqrt();
        let sb = upper.sqrt();
        let s = p.sqrt();
        let x_per_l = if s <= sa {
            1.0 / sa - 1.0 / sb
        } else if s >= sb {
            0.0
        } else {
            1.0 / s - 1.0 / sb
        };
        x_per_l * liquidity as f64
    }
}
