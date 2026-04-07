#[derive(Debug, Clone)]
pub struct Greeks {
    /// Rate of change of position value per $1 price increase.
    /// Negative when in range (LP is short volatility).
    pub delta: f64,
    /// Rate of change of delta per $1 price increase.
    pub gamma: f64,
}

/// Convert a Q64.64 sqrt_price to an `f64` price (`(sqrt_price / 2^64)^2`).
///
/// The naive `value as f64 / 2^64` loses bits for any `u128` above `2^53`.
/// To preserve precision we shift right by 32 (dropping only the lowest 32
/// fractional bits) before the float cast, then divide by `2^32`. This keeps
/// up to ~96 high bits of the input, which is well within `f64`'s mantissa
/// after the subsequent square. The intermediate sqrt_p stays finite for any
/// `u128` input including values near `u128::MAX`.
pub(crate) fn sqrt_q64_to_price(sqrt_price_q64: u128) -> f64 {
    let scaled = (sqrt_price_q64 >> 32) as f64;
    let sqrt_p = scaled / ((1u64 << 32) as f64);
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
    let price = sqrt_q64_to_price(sqrt_price_q64);
    let sqrt_p = price.sqrt();

    let price_lower = sqrt_q64_to_price(orca_whirlpools_core::tick_index_to_sqrt_price(tick_lower));
    let price_upper = sqrt_q64_to_price(orca_whirlpools_core::tick_index_to_sqrt_price(tick_upper));

    if price < price_lower || price > price_upper {
        return Greeks {
            delta: 0.0,
            gamma: 0.0,
        };
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
    fn test_sqrt_q64_to_price_precision_at_extremes() {
        // u128::MAX / 2 must convert to a finite positive f64.
        let p = sqrt_q64_to_price(u128::MAX / 2);
        assert!(
            p.is_finite() && p > 0.0,
            "expected finite positive, got {p}"
        );

        // sqrt_price = 2^64 corresponds to price = 1.0 exactly.
        let one = sqrt_q64_to_price(1u128 << 64);
        assert!((one - 1.0).abs() < 1e-12, "got {one}");

        // Hand-computed mid-range value: sqrt_price = 2 * 2^64 -> price = 4.0
        let four = sqrt_q64_to_price(2u128 << 64);
        assert!((four - 4.0).abs() / 4.0 < 1e-9, "got {four}");
    }

    #[test]
    fn test_larger_liquidity_larger_abs_delta() {
        let small = compute_greeks(100, q64_at_tick(0), -100, 100);
        let large = compute_greeks(1_000_000, q64_at_tick(0), -100, 100);
        assert!(large.delta.abs() > small.delta.abs());
    }
}
