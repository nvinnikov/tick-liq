//! Pure-Rust CLMM token amount computation.
//!
//! The `compute_token_amounts` function in `crate::analytics::amounts` uses
//! `orca_whirlpools_core` (fixed-point arithmetic on u128) for maximum
//! precision.  This module provides a floating-point version that has **zero**
//! external dependencies and is therefore easy to unit-test in isolation.
//!
//! Use this for analytics/display purposes.  For on-chain or high-precision
//! rebalance math prefer the fixed-point version in `analytics::amounts`.

/// Token amounts as floating-point (prices and liquidity in their natural units).
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub struct TokenAmountsF64 {
    /// Amount of token A (the "base" token).
    pub amount_a: f64,
    /// Amount of token B (the "quote" token).
    pub amount_b: f64,
}

/// Convert a Q64.64 sqrt_price to `f64`.
///
/// Mirrors the logic in `math::sqrt_price::sqrt_q64_to_price` but returns the
/// sqrt value so callers can use it directly in CLMM formulas.
#[allow(dead_code)]
#[inline]
fn sqrt_price_q64_to_f64(sqrt_price_x64: u128) -> f64 {
    // Shift right by 32 to avoid losing bits in the u128→f64 cast, then
    // divide by 2^32 to recover the true fractional value.
    let scaled = (sqrt_price_x64 >> 32) as f64;
    scaled / ((1u64 << 32) as f64)
}

/// Compute token amounts for a CLMM position using pure floating-point arithmetic.
///
/// Parameters match the on-chain account layout:
/// - `liquidity`:       position's active liquidity (u128)
/// - `sqrt_price_x64`: pool's current sqrt price in Q64.64 (u128)
/// - `tick_lower` / `tick_upper`: position range bounds (i32 tick indices)
///
/// Returns `(amount_a, amount_b)` per the CLMM formulas:
/// - P < Pa  → x = L·(1/√Pa − 1/√Pb),  y = 0
/// - P > Pb  → x = 0,                    y = L·(√Pb − √Pa)
/// - Pa≤P≤Pb → x = L·(1/√P  − 1/√Pb),  y = L·(√P  − √Pa)
#[allow(dead_code)]
pub fn compute_token_amounts(
    liquidity: u128,
    sqrt_price_x64: u128,
    tick_lower: i32,
    tick_upper: i32,
) -> TokenAmountsF64 {
    let l = liquidity as f64;

    // Tick → sqrt_price in Q64.64: sqrt(1.0001^tick) * 2^64
    // We compute it as f64 directly.
    let sqrt_pa = 1.0001f64.powi(tick_lower).sqrt();
    let sqrt_pb = 1.0001f64.powi(tick_upper).sqrt();
    let sqrt_p = sqrt_price_q64_to_f64(sqrt_price_x64);

    if sqrt_p < sqrt_pa {
        // Price below range: all token A.
        let amount_a = l * (1.0 / sqrt_pa - 1.0 / sqrt_pb);
        TokenAmountsF64 {
            amount_a: amount_a.max(0.0),
            amount_b: 0.0,
        }
    } else if sqrt_p >= sqrt_pb {
        // Price above range: all token B.
        let amount_b = l * (sqrt_pb - sqrt_pa);
        TokenAmountsF64 {
            amount_a: 0.0,
            amount_b: amount_b.max(0.0),
        }
    } else {
        // Price in range: both tokens.
        let amount_a = l * (1.0 / sqrt_p - 1.0 / sqrt_pb);
        let amount_b = l * (sqrt_p - sqrt_pa);
        TokenAmountsF64 {
            amount_a: amount_a.max(0.0),
            amount_b: amount_b.max(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a Q64.64 sqrt_price from a tick index (matches what orca_whirlpools_core does).
    fn sqrt_price_x64_at_tick(tick: i32) -> u128 {
        let sqrt_price_f64 = 1.0001f64.powi(tick).sqrt();
        // Multiply by 2^64 to get Q64.64.
        let scale = (1u128 << 64) as f64;
        (sqrt_price_f64 * scale) as u128
    }

    #[test]
    fn test_price_below_range_all_token_a() {
        let amounts = compute_token_amounts(
            1_000_000,
            sqrt_price_x64_at_tick(50), // price below [100, 200]
            100,
            200,
        );
        assert!(amounts.amount_a > 0.0, "token A should be > 0 below range");
        assert_eq!(amounts.amount_b, 0.0, "token B should be 0 below range");
    }

    #[test]
    fn test_price_above_range_all_token_b() {
        let amounts = compute_token_amounts(
            1_000_000,
            sqrt_price_x64_at_tick(300), // price above [100, 200]
            100,
            200,
        );
        assert_eq!(amounts.amount_a, 0.0, "token A should be 0 above range");
        assert!(amounts.amount_b > 0.0, "token B should be > 0 above range");
    }

    #[test]
    fn test_price_in_range_has_both_tokens() {
        let amounts = compute_token_amounts(
            1_000_000,
            sqrt_price_x64_at_tick(150), // in range [100, 200]
            100,
            200,
        );
        assert!(amounts.amount_a > 0.0, "token A should be > 0 in range");
        assert!(amounts.amount_b > 0.0, "token B should be > 0 in range");
    }

    #[test]
    fn test_zero_liquidity_returns_zero() {
        let amounts = compute_token_amounts(0, sqrt_price_x64_at_tick(150), 100, 200);
        assert_eq!(amounts.amount_a, 0.0);
        assert_eq!(amounts.amount_b, 0.0);
    }

    #[test]
    fn test_amounts_non_negative() {
        // Invariant: amounts are always >= 0 across the full tick range.
        for tick in [-100, 0, 100, 150, 200, 300] {
            let amounts =
                compute_token_amounts(1_000_000, sqrt_price_x64_at_tick(tick), 100, 200);
            assert!(
                amounts.amount_a >= 0.0,
                "amount_a negative at tick {tick}: {}",
                amounts.amount_a
            );
            assert!(
                amounts.amount_b >= 0.0,
                "amount_b negative at tick {tick}: {}",
                amounts.amount_b
            );
        }
    }

    #[test]
    fn test_at_lower_boundary_mostly_token_a() {
        // At exactly the lower tick the position should be almost all token A.
        let amounts = compute_token_amounts(
            1_000_000,
            sqrt_price_x64_at_tick(100), // at lower bound
            100,
            200,
        );
        assert!(
            amounts.amount_a > amounts.amount_b,
            "at lower bound A ({}) should exceed B ({})",
            amounts.amount_a,
            amounts.amount_b
        );
    }
}
