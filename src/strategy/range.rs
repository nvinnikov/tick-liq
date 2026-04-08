//! Range optimizer for concentrated-liquidity positions.
//!
//! Given a current price, a volatility estimate, and a tick-spacing
//! constraint, pick a `[lower_tick, upper_tick]` range for a new LP
//! position. Strategies are pluggable via the [`RangeStrategy`] trait so
//! the rebalance signal generator (task #13) and the backtest engine
//! (task #20) can swap between them without touching call sites.
//!
//! ## What "volatility" means here
//!
//! The input is a *relative-return* standard deviation, σ, over whatever
//! lookback window the caller cares about (hourly log-return stdev,
//! session ATR divided by mid, etc.). We treat σ as a fractional width
//! (e.g. `σ = 0.05` ⇒ 5 %) and scale it by `k` to get the range
//! half-width. This module does not compute σ itself — that belongs to
//! the fee tracker / price-history layer. σ must be finite and positive.
//!
//! ## Capital efficiency formula
//!
//! We report capital efficiency as the V3 "concentration multiple":
//!
//! ```text
//! CE = 1 / (1 − √(Pa / Pb))
//! ```
//!
//! This is the ratio of virtual liquidity in the range `[Pa, Pb]` to the
//! same capital spread across `[0, ∞)` (Uniswap V2). It depends only on
//! the range endpoints, is monotone in tightness (narrower range ⇒ higher
//! CE), and collapses to 1 for `Pa = 0`. It is *not* an expected fee
//! yield — that requires a volume / fee-rate model and belongs in the
//! backtest engine.

use anyhow::{bail, Result};

use crate::math::tick::{align_tick_to_spacing, price_to_tick, TICK_BASE};

/// Context passed to every range strategy.
#[derive(Debug, Clone, Copy)]
pub struct RangeContext {
    /// Current pool price (display units, quote per base).
    pub current_price: f64,
    /// Relative-return volatility estimate (fractional, e.g. 0.05 = 5 %).
    pub volatility: f64,
    /// Pool tick spacing (e.g. 64 for an Orca SOL/USDC 0.05 % pool).
    pub tick_spacing: u16,
    /// Base-token decimals (used by `price_to_tick`).
    pub decimals_a: u8,
    /// Quote-token decimals (used by `price_to_tick`).
    pub decimals_b: u8,
}

/// A range recommendation aligned to pool tick spacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeRecommendation {
    pub lower_tick: i32,
    pub upper_tick: i32,
    /// Expected capital efficiency (V3 concentration multiple), stored as
    /// parts-per-million so the type can derive `Eq` for golden tests.
    /// Divide by 1e6 for a plain ratio, or call
    /// [`RangeRecommendation::capital_efficiency`].
    pub expected_capital_efficiency_ppm: u32,
}

impl RangeRecommendation {
    pub fn capital_efficiency(&self) -> f64 {
        self.expected_capital_efficiency_ppm as f64 / 1e6
    }
}

/// Pluggable range-selection strategy.
pub trait RangeStrategy {
    fn recommend(&self, ctx: &RangeContext) -> Result<RangeRecommendation>;
}

/// Fixed relative half-width around the current price, ignoring volatility.
/// `half_width_frac = 0.1` ⇒ `[P · 0.9, P · 1.1]` before tick alignment.
#[derive(Debug, Clone, Copy)]
pub struct FixedWidth {
    pub half_width_frac: f64,
}

impl RangeStrategy for FixedWidth {
    fn recommend(&self, ctx: &RangeContext) -> Result<RangeRecommendation> {
        validate_ctx(ctx)?;
        if !self.half_width_frac.is_finite()
            || self.half_width_frac <= 0.0
            || self.half_width_frac >= 1.0
        {
            bail!(
                "half_width_frac must be finite and in (0, 1), got {}",
                self.half_width_frac
            );
        }
        let lower = ctx.current_price * (1.0 - self.half_width_frac);
        let upper = ctx.current_price * (1.0 + self.half_width_frac);
        finalize(ctx, lower, upper)
    }
}

/// Symmetric range sized as `k · σ` fractional half-width. Stretches
/// proportionally to recent volatility: higher σ ⇒ wider range ⇒ lower
/// CE but less frequent out-of-range time.
#[derive(Debug, Clone, Copy)]
pub struct VolatilityScaled {
    /// Multiplier on σ. Typical values: 1.5–3.0 for ≈68–99 % coverage
    /// assuming log-normal returns over the lookback.
    pub k: f64,
}

impl RangeStrategy for VolatilityScaled {
    fn recommend(&self, ctx: &RangeContext) -> Result<RangeRecommendation> {
        validate_ctx(ctx)?;
        if !self.k.is_finite() || self.k <= 0.0 {
            bail!("k must be finite and positive, got {}", self.k);
        }
        let half = self.k * ctx.volatility;
        if half >= 1.0 {
            bail!("k * volatility must be < 1.0 (got {})", half);
        }
        let lower = ctx.current_price * (1.0 - half);
        let upper = ctx.current_price * (1.0 + half);
        finalize(ctx, lower, upper)
    }
}

/// Asymmetric range — different up/down multipliers on σ. Use when you
/// want to express a directional view (e.g. `up_k > down_k` if you expect
/// upward drift) while still scaling with volatility.
#[derive(Debug, Clone, Copy)]
pub struct AsymmetricSkewed {
    pub up_k: f64,
    pub down_k: f64,
}

impl RangeStrategy for AsymmetricSkewed {
    fn recommend(&self, ctx: &RangeContext) -> Result<RangeRecommendation> {
        validate_ctx(ctx)?;
        for (name, v) in [("up_k", self.up_k), ("down_k", self.down_k)] {
            if !v.is_finite() || v <= 0.0 {
                bail!("{} must be finite and positive, got {}", name, v);
            }
        }
        let down = self.down_k * ctx.volatility;
        let up = self.up_k * ctx.volatility;
        if down >= 1.0 {
            bail!("down_k * volatility must be < 1.0 (got {})", down);
        }
        let lower = ctx.current_price * (1.0 - down);
        let upper = ctx.current_price * (1.0 + up);
        finalize(ctx, lower, upper)
    }
}

fn validate_ctx(ctx: &RangeContext) -> Result<()> {
    if !ctx.current_price.is_finite() || ctx.current_price <= 0.0 {
        bail!(
            "current_price must be finite and positive, got {}",
            ctx.current_price
        );
    }
    if !ctx.volatility.is_finite() || ctx.volatility <= 0.0 {
        bail!(
            "volatility must be finite and positive, got {}",
            ctx.volatility
        );
    }
    if ctx.tick_spacing == 0 {
        bail!("tick_spacing must be > 0");
    }
    Ok(())
}

/// Convert a `[lower_price, upper_price]` pair into a tick-aligned
/// recommendation. The lower price is floored to the spacing; the upper
/// price is ceilinged to the next spacing multiple so tick alignment
/// never shrinks the intended range.
fn finalize(ctx: &RangeContext, lower_price: f64, upper_price: f64) -> Result<RangeRecommendation> {
    let raw_lower = price_to_tick(lower_price, ctx.decimals_a, ctx.decimals_b)?;
    let raw_upper = price_to_tick(upper_price, ctx.decimals_a, ctx.decimals_b)?;

    let lower_tick = align_tick_to_spacing(raw_lower, ctx.tick_spacing)?;
    let s = ctx.tick_spacing as i32;
    // Ceiling alignment: bump up by (spacing − 1) then floor.
    let upper_tick = align_tick_to_spacing(raw_upper + s - 1, ctx.tick_spacing)?;

    if upper_tick <= lower_tick {
        bail!(
            "degenerate range after tick alignment: lower={}, upper={}",
            lower_tick,
            upper_tick
        );
    }

    // CE = 1 / (1 − √(Pa/Pb)). Token decimals cancel in the ratio, so we
    // can use raw tick prices (`1.0001^tick`) directly.
    let pa_over_pb = TICK_BASE.powi(lower_tick - upper_tick);
    let ce = 1.0 / (1.0 - pa_over_pb.sqrt());
    // Clamp to u32::MAX ppm to handle absurdly tight ranges.
    let ce_ppm = (ce * 1e6).clamp(0.0, u32::MAX as f64) as u32;

    Ok(RangeRecommendation {
        lower_tick,
        upper_tick,
        expected_capital_efficiency_ppm: ce_ppm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RangeContext {
        RangeContext {
            current_price: 100.0,
            volatility: 0.05,
            tick_spacing: 64,
            decimals_a: 9, // SOL
            decimals_b: 6, // USDC
        }
    }

    #[test]
    fn fixed_width_produces_valid_range() {
        let r = FixedWidth {
            half_width_frac: 0.1,
        }
        .recommend(&ctx())
        .unwrap();
        assert!(r.lower_tick < r.upper_tick);
        assert!(r.capital_efficiency() > 1.0);
    }

    #[test]
    fn narrower_width_has_higher_ce() {
        let wide = FixedWidth {
            half_width_frac: 0.2,
        }
        .recommend(&ctx())
        .unwrap();
        let tight = FixedWidth {
            half_width_frac: 0.05,
        }
        .recommend(&ctx())
        .unwrap();
        assert!(tight.capital_efficiency() > wide.capital_efficiency());
    }

    #[test]
    fn volatility_scaled_widens_with_sigma() {
        let mut c = ctx();
        c.volatility = 0.02;
        let tight = VolatilityScaled { k: 2.0 }.recommend(&c).unwrap();
        c.volatility = 0.10;
        let wide = VolatilityScaled { k: 2.0 }.recommend(&c).unwrap();
        assert!(wide.upper_tick - wide.lower_tick > tight.upper_tick - tight.lower_tick);
        assert!(tight.capital_efficiency() > wide.capital_efficiency());
    }

    #[test]
    fn asymmetric_skew_is_asymmetric_around_current_price() {
        let r = AsymmetricSkewed {
            up_k: 3.0,
            down_k: 1.0,
        }
        .recommend(&ctx())
        .unwrap();
        // raw tick price × 10^(dec_a - dec_b) = display price.
        let decimal_shift = 1e3;
        let pa = TICK_BASE.powi(r.lower_tick) * decimal_shift;
        let pb = TICK_BASE.powi(r.upper_tick) * decimal_shift;
        let down = 100.0 - pa;
        let up = pb - 100.0;
        assert!(down > 0.0 && up > 0.0);
        assert!(up > down);
    }

    #[test]
    fn ticks_aligned_to_spacing() {
        let r = FixedWidth {
            half_width_frac: 0.1,
        }
        .recommend(&ctx())
        .unwrap();
        assert_eq!(r.lower_tick % 64, 0);
        assert_eq!(r.upper_tick % 64, 0);
    }

    #[test]
    fn rejects_invalid_inputs() {
        let mut c = ctx();
        c.current_price = 0.0;
        assert!(FixedWidth {
            half_width_frac: 0.1
        }
        .recommend(&c)
        .is_err());

        let mut c = ctx();
        c.volatility = -1.0;
        assert!(VolatilityScaled { k: 2.0 }.recommend(&c).is_err());

        let c = ctx();
        assert!(FixedWidth {
            half_width_frac: 1.5
        }
        .recommend(&c)
        .is_err());
        assert!(VolatilityScaled { k: 30.0 }.recommend(&c).is_err());
    }
}
