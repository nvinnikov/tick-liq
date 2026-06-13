//! Q64.64 sqrt-price helpers (pure).

/// Convert a Q64.64 sqrt_price to an `f64` price (`(sqrt_price / 2^64)^2`).
///
/// The naive `value as f64 / 2^64` loses bits for any `u128` above `2^53`.
/// To preserve precision we shift right by 32 (dropping only the lowest 32
/// fractional bits) before the float cast, then divide by `2^32`. This keeps
/// up to ~96 high bits of the input, which is well within `f64`'s mantissa
/// after the subsequent square. The intermediate sqrt_p stays finite for any
/// `u128` input including values near `u128::MAX`.
pub fn sqrt_q64_to_price(sqrt_price_q64: u128) -> f64 {
    let scaled = (sqrt_price_q64 >> 32) as f64;
    let sqrt_p = scaled / ((1u64 << 32) as f64);
    sqrt_p * sqrt_p
}

/// Convert a Q64.64 sqrt_price to a human-unit ("UI") price: token B per
/// token A with both sides decimal-adjusted.
///
/// `sqrt_q64_to_price` yields the *raw* price (token-B base units per
/// token-A base unit); multiplying by `10^(decimals_a - decimals_b)`
/// converts it to the price humans quote (e.g. SOL/USDC 9/6: raw 0.084 →
/// $84). Every display, IL, P&L or entry-price comparison must use the same
/// unit space — mixing raw and UI prices is the BUG-qr9 class of defect.
pub fn sqrt_q64_to_ui_price(sqrt_price_q64: u128, decimals_a: u8, decimals_b: u8) -> f64 {
    sqrt_q64_to_price(sqrt_price_q64) * 10f64.powi(decimals_a as i32 - decimals_b as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
