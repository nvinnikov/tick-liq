#[derive(Debug, Clone)]
pub struct Greeks {
    /// Rate of change of position value per $1 price increase.
    /// Negative when in range (LP is short volatility).
    pub delta: f64,
    /// Rate of change of delta per $1 price increase.
    pub gamma: f64,
}

/// Convert Q64.64 sqrt_price to f64 price.
pub fn sqrt_price_q64_to_price(sqrt_price_q64: u128) -> f64 {
    let sqrt_p = sqrt_price_q64 as f64 / (1u128 << 64) as f64;
    sqrt_p * sqrt_p
}

/// Compute position Greeks.
///
/// Returns delta=0, gamma=0 when price is outside the range.
pub fn compute_greeks(
    liquidity: u128,
    sqrt_price_q64: u128,
    tick_lower: i32,
    tick_upper: i32,
) -> Greeks {
    let sqrt_p = (sqrt_price_q64 as f64) / (1u128 << 64) as f64;
    let price = sqrt_p * sqrt_p;

    let price_lower = 1.0001f64.powi(tick_lower);
    let price_upper = 1.0001f64.powi(tick_upper);

    if price < price_lower || price > price_upper {
        return Greeks { delta: 0.0, gamma: 0.0 };
    }

    let l = liquidity as f64;

    // delta = -L / (2 * sqrt(P) * P)  [from CLAUDE.md]
    let delta = -l / (2.0 * sqrt_p * price);

    // gamma = L / (2 * P^(5/2))
    let gamma = l / (2.0 * price * price * sqrt_p);

    Greeks { delta, gamma }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q64_at_tick(tick: i32) -> u128 {
        let sqrt_p = (1.0001f64.powi(tick)).sqrt();
        (sqrt_p * (1u128 << 64) as f64) as u128
    }

    #[test]
    fn test_delta_negative_when_in_range() {
        let greeks = compute_greeks(1_000_000, q64_at_tick(0), -100, 100);
        assert!(greeks.delta < 0.0, "delta should be negative in range");
    }

    #[test]
    fn test_gamma_positive_when_in_range() {
        let greeks = compute_greeks(1_000_000, q64_at_tick(0), -100, 100);
        assert!(greeks.gamma > 0.0, "gamma should be positive in range");
    }

    #[test]
    fn test_delta_zero_above_range() {
        let greeks = compute_greeks(1_000_000, q64_at_tick(200), -100, 100);
        assert_eq!(greeks.delta, 0.0);
    }

    #[test]
    fn test_larger_liquidity_larger_abs_delta() {
        let small = compute_greeks(100, q64_at_tick(0), -100, 100);
        let large = compute_greeks(1_000_000, q64_at_tick(0), -100, 100);
        assert!(large.delta.abs() > small.delta.abs());
    }
}
