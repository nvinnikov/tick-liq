//! Liquidity ↔ token amount conversions for concentrated liquidity positions.
//!
//! Implements the standard Uniswap V3 / Whirlpool position arithmetic
//! described in `CLAUDE.md` (Math Reference). Inputs are sqrt-prices in
//! Q64.64 fixed-point (`u128`); outputs are raw on-chain token amounts
//! (`u64`) or position liquidity (`u128`).
//!
//! For the *forward* direction (`liquidity → amounts`) we delegate to
//! `orca_whirlpools_core::try_get_amount_delta_a/b`, the canonical Whirlpool
//! routine, so the result is bit-exact with what the pool will compute on
//! chain.
//!
//! For the *inverse* direction (`amounts → liquidity`) the reference crate
//! does not expose a helper, so we implement the closed-form formulas here:
//!
//!   L_x = x · sqrt(P_a) · sqrt(P_b) / (sqrt(P_b) − sqrt(P_a))   (token-x side)
//!   L_y = y / (sqrt(P_b) − sqrt(P_a))                            (token-y side)
//!
//! When the price is in range we return `min(L_x, L_y)` (the binding side).
//! When the price is outside the range, only the active side contributes.
//!
//! All multiplications widen to `u256` (via `ethnum`) to avoid intermediate
//! overflow on `Q64.64 · Q64.64 · u64` products. The final liquidity value
//! is returned as `u128`; we error out if it does not fit.

use anyhow::{anyhow, bail, Result};
use ethnum::U256;
use orca_whirlpools_core::{try_get_amount_delta_a, try_get_amount_delta_b};

/// Token amounts held in a position, in raw on-chain units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenAmounts {
    pub amount_x: u64,
    pub amount_y: u64,
}

/// Compute the (x, y) token amounts for a position with `liquidity` at the
/// current `sqrt_price`, bounded by `[sqrt_price_lower, sqrt_price_upper]`.
///
/// Handles all three regimes:
///   - `P < P_a`: only token x (price below range)
///   - `P_a ≤ P < P_b`: both tokens
///   - `P ≥ P_b`: only token y (price above range)
///
/// Errors if the bounds are non-strictly-ordered or if the underlying
/// `u64` amount calculation overflows.
pub fn get_amounts_for_liquidity(
    liquidity: u128,
    sqrt_price: u128,
    sqrt_price_lower: u128,
    sqrt_price_upper: u128,
) -> Result<TokenAmounts> {
    if sqrt_price_lower >= sqrt_price_upper {
        bail!("sqrt_price_lower must be < sqrt_price_upper");
    }

    let (x, y) = if sqrt_price < sqrt_price_lower {
        // Price below range: position is entirely token x.
        let x = try_get_amount_delta_a(sqrt_price_lower, sqrt_price_upper, liquidity, false)
            .map_err(|e| anyhow!("amount_delta_a (below range) failed: {:?}", e))?;
        (x, 0u64)
    } else if sqrt_price >= sqrt_price_upper {
        // Price above range: position is entirely token y.
        let y = try_get_amount_delta_b(sqrt_price_lower, sqrt_price_upper, liquidity, false)
            .map_err(|e| anyhow!("amount_delta_b (above range) failed: {:?}", e))?;
        (0u64, y)
    } else {
        // In range: both sides contribute.
        let x = try_get_amount_delta_a(sqrt_price, sqrt_price_upper, liquidity, false)
            .map_err(|e| anyhow!("amount_delta_a (in range) failed: {:?}", e))?;
        let y = try_get_amount_delta_b(sqrt_price_lower, sqrt_price, liquidity, false)
            .map_err(|e| anyhow!("amount_delta_b (in range) failed: {:?}", e))?;
        (x, y)
    };

    Ok(TokenAmounts {
        amount_x: x,
        amount_y: y,
    })
}

/// Liquidity supportable by `amount_x` token x alone given the price range.
///
/// Formula: `L = x · sqrt(P_a) · sqrt(P_b) / (sqrt(P_b) − sqrt(P_a)) / 2^64`
///
/// The `2^64` divisor undoes one Q64.64 fixed-point factor so the result is
/// a plain `u128` liquidity value.
pub fn get_liquidity_for_amount_x(
    sqrt_price_lower: u128,
    sqrt_price_upper: u128,
    amount_x: u64,
) -> Result<u128> {
    if sqrt_price_lower >= sqrt_price_upper {
        bail!("sqrt_price_lower must be < sqrt_price_upper");
    }
    if amount_x == 0 {
        return Ok(0);
    }

    let lower = U256::from(sqrt_price_lower);
    let upper = U256::from(sqrt_price_upper);
    let diff = upper - lower;

    // numerator = x * lower * upper  (fits in u256: u64 * u128 * u128 ≤ 2^256)
    let intermediate = U256::from(amount_x)
        .checked_mul(lower)
        .ok_or_else(|| anyhow!("L_x: x * sqrt_lower overflowed u256"))?;
    let numerator = intermediate
        .checked_mul(upper)
        .ok_or_else(|| anyhow!("L_x: (x * sqrt_lower) * sqrt_upper overflowed u256"))?;

    // L = numerator / diff / 2^64
    let l = numerator / diff;
    let l = l >> 64;

    u128::try_from(l).map_err(|_| anyhow!("L_x exceeds u128"))
}

/// Liquidity supportable by `amount_y` token y alone given the price range.
///
/// Formula: `L = y · 2^64 / (sqrt(P_b) − sqrt(P_a))`
pub fn get_liquidity_for_amount_y(
    sqrt_price_lower: u128,
    sqrt_price_upper: u128,
    amount_y: u64,
) -> Result<u128> {
    if sqrt_price_lower >= sqrt_price_upper {
        bail!("sqrt_price_lower must be < sqrt_price_upper");
    }
    if amount_y == 0 {
        return Ok(0);
    }

    let diff = U256::from(sqrt_price_upper) - U256::from(sqrt_price_lower);

    // numerator = y << 64  (u64 << 64 fits comfortably in u256)
    let numerator = U256::from(amount_y) << 64;
    let l = numerator / diff;

    u128::try_from(l).map_err(|_| anyhow!("L_y exceeds u128"))
}

/// Inverse of [`get_amounts_for_liquidity`]: given `(x, y)` and a price
/// range, return the maximum liquidity that is consistent with both sides.
///
/// In range we return `min(L_x, L_y)` because the binding (smaller) side
/// determines how much liquidity the position can actually be opened with.
/// Outside the range only one side contributes.
pub fn get_liquidity_for_amounts(
    sqrt_price: u128,
    sqrt_price_lower: u128,
    sqrt_price_upper: u128,
    amount_x: u64,
    amount_y: u64,
) -> Result<u128> {
    if sqrt_price_lower >= sqrt_price_upper {
        bail!("sqrt_price_lower must be < sqrt_price_upper");
    }

    if sqrt_price < sqrt_price_lower {
        get_liquidity_for_amount_x(sqrt_price_lower, sqrt_price_upper, amount_x)
    } else if sqrt_price >= sqrt_price_upper {
        get_liquidity_for_amount_y(sqrt_price_lower, sqrt_price_upper, amount_y)
    } else {
        let lx = get_liquidity_for_amount_x(sqrt_price, sqrt_price_upper, amount_x)?;
        let ly = get_liquidity_for_amount_y(sqrt_price_lower, sqrt_price, amount_y)?;
        Ok(lx.min(ly))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orca_whirlpools_core::tick_index_to_sqrt_price;

    fn sp(tick: i32) -> u128 {
        tick_index_to_sqrt_price(tick)
    }

    #[test]
    fn below_range_only_x() {
        let amts = get_amounts_for_liquidity(1_000_000, sp(50), sp(100), sp(200)).unwrap();
        assert!(amts.amount_x > 0);
        assert_eq!(amts.amount_y, 0);
    }

    #[test]
    fn above_range_only_y() {
        let amts = get_amounts_for_liquidity(1_000_000, sp(300), sp(100), sp(200)).unwrap();
        assert_eq!(amts.amount_x, 0);
        assert!(amts.amount_y > 0);
    }

    #[test]
    fn in_range_both_sides() {
        let amts = get_amounts_for_liquidity(1_000_000_000, sp(150), sp(100), sp(200)).unwrap();
        assert!(amts.amount_x > 0);
        assert!(amts.amount_y > 0);
    }

    #[test]
    fn invalid_bounds_error() {
        assert!(get_amounts_for_liquidity(1_000, sp(100), sp(200), sp(100)).is_err());
        assert!(get_liquidity_for_amount_x(sp(200), sp(100), 1).is_err());
        assert!(get_liquidity_for_amount_y(sp(200), sp(100), 1).is_err());
    }

    #[test]
    fn zero_amount_zero_liquidity() {
        assert_eq!(get_liquidity_for_amount_x(sp(100), sp(200), 0).unwrap(), 0);
        assert_eq!(get_liquidity_for_amount_y(sp(100), sp(200), 0).unwrap(), 0);
    }

    #[test]
    fn round_trip_in_range_close() {
        // Pick a non-trivial liquidity, derive amounts, then derive liquidity
        // back. They should agree to within a small rounding tolerance because
        // amount calculations floor.
        let l_in: u128 = 1_000_000_000_000;
        let amts = get_amounts_for_liquidity(l_in, sp(150), sp(100), sp(200)).unwrap();
        let l_out =
            get_liquidity_for_amounts(sp(150), sp(100), sp(200), amts.amount_x, amts.amount_y)
                .unwrap();
        // Floor on both sides means l_out ≤ l_in; allow a small relative drift.
        assert!(l_out <= l_in);
        assert!(l_in - l_out < l_in / 100_000); // < 0.001%
    }
}
