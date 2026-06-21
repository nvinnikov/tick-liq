//! Position Greeks (pure math).
//!
//! These formulas take prices as `f64` and have no dependency on any tick↔
//! sqrt_price encoding. Conversion from on-chain tick indices lives in
//! `crate::analytics::greeks`, which calls into this module.

#[derive(Debug, Clone)]
pub struct Greeks {
    /// Rate of change of position value per $1 price increase.
    /// Negative when in range (LP is short volatility).
    pub delta: f64,
    /// Rate of change of delta per $1 price increase.
    pub gamma: f64,
}

/// Compute position Greeks from plain prices.
///
/// Returns delta=0, gamma=0 when price is outside `[price_lower, price_upper]`.
pub fn compute_greeks_from_prices(
    liquidity: u128,
    price: f64,
    price_lower: f64,
    price_upper: f64,
) -> Greeks {
    // Degenerate / out-of-range inputs return the safe zero default (never
    // NaN/Inf). `price <= 0.0` would make `1/(2·√P·P)` non-finite, so it is
    // guarded here alongside the out-of-range check.
    if price <= 0.0 || price < price_lower || price > price_upper {
        return Greeks {
            delta: 0.0,
            gamma: 0.0,
        };
    }

    let sqrt_p = price.sqrt();
    let l = liquidity as f64;

    // delta = -L / (2 * sqrt(P) * P)
    let delta = -l / (2.0 * sqrt_p * price);

    // gamma = d(delta)/dP = 3L / (4 * P^(5/2))
    let gamma = 3.0 * l / (4.0 * price * price * sqrt_p);

    Greeks { delta, gamma }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delta_negative_when_in_range() {
        let g = compute_greeks_from_prices(1_000_000, 100.0, 80.0, 120.0);
        assert!(g.delta < 0.0);
    }

    #[test]
    fn test_gamma_positive_when_in_range() {
        let g = compute_greeks_from_prices(1_000_000, 100.0, 80.0, 120.0);
        assert!(g.gamma > 0.0);
    }

    #[test]
    fn test_delta_zero_above_range() {
        let g = compute_greeks_from_prices(1_000_000, 150.0, 80.0, 120.0);
        assert_eq!(g.delta, 0.0);
    }

    #[test]
    fn test_non_positive_price_returns_zero_greeks() {
        let g = compute_greeks_from_prices(1_000_000, 0.0, 0.0, 120.0);
        assert_eq!(g.delta, 0.0);
        assert_eq!(g.gamma, 0.0);
        assert!(g.delta.is_finite() && g.gamma.is_finite());
    }

    #[test]
    fn test_larger_liquidity_larger_abs_delta() {
        let small = compute_greeks_from_prices(100, 100.0, 80.0, 120.0);
        let large = compute_greeks_from_prices(1_000_000, 100.0, 80.0, 120.0);
        assert!(large.delta.abs() > small.delta.abs());
    }
}
