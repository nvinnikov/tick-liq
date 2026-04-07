//! Tick ↔ price conversion (Orca Whirlpool / Uniswap V3 sqrt-price model).
//!
//! In a CLMM the pool price is encoded as a *tick index* `i ∈ [MIN_TICK, MAX_TICK]`,
//! defined by `price = 1.0001^i`. The on-chain representation is the
//! square root of that price as a Q64.64 fixed-point integer
//! (`sqrt_price_x64 = floor(sqrt(price) * 2^64)`).
//!
//! This module exposes typed helpers around the bit-by-bit conversion routines
//! in [`orca_whirlpools_core`] (the canonical Rust implementation, equivalent
//! to the Whirlpool TypeScript SDK), plus convenience helpers that the
//! reference crate does not provide:
//!
//! - [`tick_to_price`] / [`price_to_tick`]: human-readable price including
//!   token-decimals adjustment.
//! - [`align_tick_to_spacing`]: floor a tick to the nearest pool tick-spacing
//!   multiple, the operation needed when opening a position around a target
//!   tick.

use anyhow::{bail, Result};
use orca_whirlpools_core::{
    sqrt_price_to_tick_index, tick_index_to_sqrt_price, MAX_TICK_INDEX, MIN_TICK_INDEX,
};

/// Inclusive lower bound of the valid tick range (Whirlpool/Uniswap V3).
pub const MIN_TICK: i32 = MIN_TICK_INDEX;
/// Inclusive upper bound of the valid tick range (Whirlpool/Uniswap V3).
pub const MAX_TICK: i32 = MAX_TICK_INDEX;

/// Base of the tick exponential: `price = TICK_BASE.powi(tick)`.
pub const TICK_BASE: f64 = 1.0001;

/// Convert a tick index to its `sqrt_price` Q64.64 representation.
///
/// Returns an error if `tick` is outside `[MIN_TICK, MAX_TICK]` — the
/// underlying algorithm only guarantees precision inside that range.
pub fn tick_to_sqrt_price_x64(tick: i32) -> Result<u128> {
    if !(MIN_TICK..=MAX_TICK).contains(&tick) {
        bail!("tick {} out of range [{}, {}]", tick, MIN_TICK, MAX_TICK);
    }
    Ok(tick_index_to_sqrt_price(tick))
}

/// Convert a `sqrt_price` Q64.64 back to its tick index (floor).
///
/// Returns an error for `sqrt_price == 0` — the reference algorithm panics
/// on zero, so we guard the input.
pub fn sqrt_price_x64_to_tick(sqrt_price_x64: u128) -> Result<i32> {
    if sqrt_price_x64 == 0 {
        bail!("sqrt_price_x64 must be > 0");
    }
    Ok(sqrt_price_to_tick_index(sqrt_price_x64))
}

/// Convert a tick to a human-readable price ratio (token B per 1 token A) using
/// the token decimals. Raw on-chain ratio is `1.0001^tick`; we then scale by
/// `10^(decimals_a - decimals_b)` so the result is in display units.
pub fn tick_to_price(tick: i32, decimals_a: u8, decimals_b: u8) -> Result<f64> {
    if !(MIN_TICK..=MAX_TICK).contains(&tick) {
        bail!("tick {} out of range [{}, {}]", tick, MIN_TICK, MAX_TICK);
    }
    let raw = TICK_BASE.powi(tick);
    let decimal_shift = 10f64.powi(decimals_a as i32 - decimals_b as i32);
    Ok(raw * decimal_shift)
}

/// Inverse of [`tick_to_price`]: convert a display-unit price into the
/// nearest tick index (floor).
///
/// Returns an error for non-positive or non-finite prices, or if the
/// resulting tick falls outside `[MIN_TICK, MAX_TICK]`.
pub fn price_to_tick(price: f64, decimals_a: u8, decimals_b: u8) -> Result<i32> {
    if !price.is_finite() || price <= 0.0 {
        bail!("price must be finite and positive, got {}", price);
    }
    let decimal_shift = 10f64.powi(decimals_a as i32 - decimals_b as i32);
    let raw = price / decimal_shift;
    let tick_f = raw.ln() / TICK_BASE.ln();
    let tick = tick_f.floor() as i32;
    if !(MIN_TICK..=MAX_TICK).contains(&tick) {
        bail!(
            "computed tick {} out of range [{}, {}]",
            tick,
            MIN_TICK,
            MAX_TICK
        );
    }
    Ok(tick)
}

/// Floor `tick` to the nearest multiple of `spacing` (matching how Orca
/// initializes positions). `spacing` must be positive.
///
/// Uses Euclidean division so the result is always `<= tick` and a multiple
/// of `spacing`, even for negative ticks.
pub fn align_tick_to_spacing(tick: i32, spacing: u16) -> Result<i32> {
    if spacing == 0 {
        bail!("tick spacing must be > 0");
    }
    let s = spacing as i32;
    Ok(tick.div_euclid(s) * s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_zero_sqrt_price_is_q64_one() {
        // 1.0001^0 = 1, sqrt(1) = 1, in Q64.64 that's 1 << 64.
        let sp = tick_to_sqrt_price_x64(0).unwrap();
        assert_eq!(sp, 1u128 << 64);
    }

    #[test]
    fn round_trip_zero() {
        let sp = tick_to_sqrt_price_x64(0).unwrap();
        assert_eq!(sqrt_price_x64_to_tick(sp).unwrap(), 0);
    }

    #[test]
    fn tick_to_price_decimal_adjustment() {
        // Tick 0 → raw 1.0; SOL(9)/USDC(6) → display = 1.0 * 10^(9-6) = 1000.
        let p = tick_to_price(0, 9, 6).unwrap();
        assert!((p - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn align_tick_floors_negative() {
        assert_eq!(align_tick_to_spacing(-5, 8).unwrap(), -8);
        assert_eq!(align_tick_to_spacing(5, 8).unwrap(), 0);
        assert_eq!(align_tick_to_spacing(16, 8).unwrap(), 16);
        assert_eq!(align_tick_to_spacing(17, 8).unwrap(), 16);
    }

    #[test]
    fn align_tick_zero_spacing_errors() {
        assert!(align_tick_to_spacing(10, 0).is_err());
    }

    #[test]
    fn out_of_range_tick_errors() {
        assert!(tick_to_sqrt_price_x64(MAX_TICK + 1).is_err());
        assert!(tick_to_sqrt_price_x64(MIN_TICK - 1).is_err());
    }

    #[test]
    fn zero_sqrt_price_errors() {
        assert!(sqrt_price_x64_to_tick(0).is_err());
    }
}
