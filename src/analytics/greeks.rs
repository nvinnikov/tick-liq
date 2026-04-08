//! Orca-aware orchestration over `crate::math::greeks`.
//!
//! Converts on-chain tick indices to prices via `orca_whirlpools_core`, then
//! delegates to the pure-math greeks formulas.

pub use crate::math::greeks::Greeks;
pub use crate::math::sqrt_price::sqrt_q64_to_price;

use crate::math::greeks::compute_greeks_from_prices;

/// Compute position Greeks from on-chain inputs.
///
/// Returns delta=0, gamma=0 when price is outside the range.
pub fn compute_greeks(
    liquidity: u128,
    sqrt_price_q64: u128,
    tick_lower: i32,
    tick_upper: i32,
) -> Greeks {
    let price = sqrt_q64_to_price(sqrt_price_q64);
    let price_lower = sqrt_q64_to_price(orca_whirlpools_core::tick_index_to_sqrt_price(tick_lower));
    let price_upper = sqrt_q64_to_price(orca_whirlpools_core::tick_index_to_sqrt_price(tick_upper));

    compute_greeks_from_prices(liquidity, price, price_lower, price_upper)
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
}
