//! Q64.64 sqrt-price helpers (pure).

/// Convert a Q64.64 sqrt_price to an `f64` price (`(sqrt_price / 2^64)^2`).
///
/// Shift-before-cast, splitting the `u128` into its integer and fractional
/// halves around the Q64.64 point so neither cast is a raw large-`u128`→`f64`:
/// `sqrt_p = (value >> 64) + (value & (2^64-1)) / 2^64`. This keeps the full
/// relative precision of both halves.
///
/// The previous implementation pre-shifted by only 32 and divided by `2^32`,
/// which **floored** the input to multiples of `2^32` and produced up to ~50%
/// error for very small sqrt prices (low-tick pools near MIN_TICK). Splitting
/// at the actual Q64.64 boundary fixes that while staying finite for the whole
/// `u128` range up to `u128::MAX`.
pub fn sqrt_q64_to_price(sqrt_price_q64: u128) -> f64 {
    const TWO_POW_64: f64 = 18_446_744_073_709_551_616.0; // 2^64
    let int_part = (sqrt_price_q64 >> 64) as u64; // sqrt price integer part
    let frac_part = (sqrt_price_q64 & u64::MAX as u128) as u64; // Q.64 fraction
    let sqrt_p = int_part as f64 + frac_part as f64 / TWO_POW_64;
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

    #[test]
    fn test_sqrt_q64_to_price_accurate_at_low_sqrt_price() {
        // Regression: the old >> 32 pre-shift floored small inputs to 2^32
        // multiples, giving ~49% error here. Direct cast must be exact to f64.
        let q: u128 = 6_000_000_000; // ~ tick -437k, a valid low-price pool
        let expected = (q as f64 / 18_446_744_073_709_551_616.0).powi(2);
        let got = sqrt_q64_to_price(q);
        assert!(
            ((got - expected) / expected).abs() < 1e-12,
            "expected {expected:e}, got {got:e}"
        );
        // And sanity vs the closed form: ~1.058e-19.
        assert!(
            (got - 1.0578759e-19).abs() / 1.0578759e-19 < 1e-3,
            "got {got:e}"
        );
    }
}
