//! Real P&L engine: fees earned minus impermanent loss.
//!
//! Composes [`crate::math::il::impermanent_loss`] with an accumulated-fee
//! input to produce a single `net` figure (in quote units) that represents
//! what the LP has actually made versus a HODL baseline.
//!
//! ```text
//! V_hodl_now  = entry_x * P_now + entry_y
//! il_quote    = il_fraction * V_hodl_now            (≤ 0)
//! fees_earned = fees.base * P_now + fees.quote      (≥ 0)
//! net         = fees_earned + il_quote
//!             = V_lp_now − (V_hodl_now − fees_earned)
//! ```
//!
//! Both sides of the comparison are expressed in **present-value** quote
//! units: fees are present-value (both legs converted at the current
//! price), and IL is anchored to the HODL value *at the current price*.
//! This is the mathematically clean identity "what I have vs what I would
//! have had if I'd just held the entry composition," and is exactly the
//! signal the rebalance strategy layer needs — "did my fees cover my IL
//! *right now*?"
//!
//! Sign convention: `il_quote ≤ 0`, `fees_earned ≥ 0`, `net` can be either
//! sign — strictly positive exactly when fees have outpaced IL.
//!
//! ## Fee source
//!
//! This module is intentionally decoupled from the fee tracker (task #10).
//! It accepts an accumulated [`FeeDelta`] — base and quote token amounts
//! earned since position open — as a plain input, so the P&L engine can be
//! tested and consumed independently of whatever source wires up live fee
//! accounting later. When #10 lands, its output type can either become or
//! produce a `FeeDelta` without any change to this module.
//!
//! All prices and amounts are `f64` in *display* units, matching the
//! convention in [`crate::math::il`] and [`crate::math::greeks`].

use anyhow::{bail, Result};

use crate::math::il::impermanent_loss;

/// Fees accumulated by the position since entry, split by leg.
///
/// Both fields are in display units of the respective token (e.g. SOL and
/// USDC). They must be finite and non-negative.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FeeDelta {
    /// Base-token fees earned since entry.
    pub base: f64,
    /// Quote-token fees earned since entry.
    pub quote: f64,
}

impl FeeDelta {
    pub const ZERO: Self = FeeDelta {
        base: 0.0,
        quote: 0.0,
    };

    /// Total fee value in quote units at the given price.
    #[inline]
    fn to_quote(self, price: f64) -> f64 {
        self.base * price + self.quote
    }
}

/// A point-in-time P&L snapshot for a single LP position.
///
/// All figures are in quote units (e.g. USDC) at the current price.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PnlSnapshot {
    /// Fees earned since entry, converted to quote at the current price.
    /// Non-negative.
    pub fees_earned: f64,
    /// Impermanent loss in quote units (non-positive). Equals
    /// `V_lp_now − V_hodl_now` where `V_hodl_now = entry_x * P_now +
    /// entry_y`, i.e. the present-value gap between the LP's current
    /// composition and the entry composition held to now.
    pub il_quote: f64,
    /// `fees_earned + il_quote` — the LP's realized edge versus HODL.
    pub net: f64,
}

/// Inputs required to compute a P&L snapshot.
#[derive(Debug, Clone, Copy)]
pub struct PnlInput {
    /// Price at which the position was opened (display units).
    pub entry_price: f64,
    /// Current pool price (display units).
    pub current_price: f64,
    /// Lower bound of the LP range (display units).
    pub lower_price: f64,
    /// Upper bound of the LP range (display units).
    pub upper_price: f64,
    /// Amount of base token deposited at entry (display units).
    pub entry_x: f64,
    /// Amount of quote token deposited at entry (display units).
    pub entry_y: f64,
    /// Accumulated fees earned since entry.
    pub fees: FeeDelta,
}

/// Compute a P&L snapshot from the given inputs.
///
/// # Errors
/// Returns an error on any non-finite / non-positive price, inverted range,
/// negative/non-finite fee or entry amounts, or a degenerate entry
/// composition (both legs zero).
pub fn compute_pnl(input: PnlInput) -> Result<PnlSnapshot> {
    let PnlInput {
        entry_price,
        current_price,
        lower_price,
        upper_price,
        entry_x,
        entry_y,
        fees,
    } = input;

    for (name, v) in [
        ("entry_x", entry_x),
        ("entry_y", entry_y),
        ("fees.base", fees.base),
        ("fees.quote", fees.quote),
    ] {
        if !v.is_finite() || v < 0.0 {
            bail!("{} must be finite and non-negative, got {}", name, v);
        }
    }
    if entry_x == 0.0 && entry_y == 0.0 {
        bail!("entry composition (entry_x, entry_y) is all zero");
    }

    // Delegates all price validation to impermanent_loss.
    let il_fraction = impermanent_loss(entry_price, current_price, lower_price, upper_price)?;

    let v_hodl_now = entry_x * current_price + entry_y;
    let fees_earned = fees.to_quote(current_price);
    let il_quote = il_fraction * v_hodl_now;
    let net = fees_earned + il_quote;

    Ok(PnlSnapshot {
        fees_earned,
        il_quote,
        net,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Entry composition of an LP at price 100 with range [80, 125] and
    // L=1000. Derived once from the standard Uniswap V3 amounts formulas.
    //   sa = sqrt(80),  sb = sqrt(125),  s = sqrt(100)
    //   x = L*(1/s - 1/sb),  y = L*(s - sa)
    const L: f64 = 1_000.0;

    fn entry_amounts(p: f64, lower: f64, upper: f64, l: f64) -> (f64, f64) {
        let sa = lower.sqrt();
        let sb = upper.sqrt();
        let s = p.sqrt();
        (l * (1.0 / s - 1.0 / sb), l * (s - sa))
    }

    fn base_input() -> PnlInput {
        let (x, y) = entry_amounts(100.0, 80.0, 125.0, L);
        PnlInput {
            entry_price: 100.0,
            current_price: 100.0,
            lower_price: 80.0,
            upper_price: 125.0,
            entry_x: x,
            entry_y: y,
            fees: FeeDelta::ZERO,
        }
    }

    #[test]
    fn zero_pnl_at_entry_with_no_fees() {
        let snap = compute_pnl(base_input()).unwrap();
        assert_eq!(snap.fees_earned, 0.0);
        assert!(snap.il_quote.abs() < 1e-9);
        assert!(snap.net.abs() < 1e-9);
    }

    #[test]
    fn fees_only_when_price_unchanged() {
        let mut input = base_input();
        input.fees = FeeDelta {
            base: 0.1,
            quote: 5.0,
        };
        let snap = compute_pnl(input).unwrap();
        // 0.1 * 100 + 5 = 15
        assert!((snap.fees_earned - 15.0).abs() < 1e-9);
        assert!(snap.il_quote.abs() < 1e-9);
        assert!((snap.net - 15.0).abs() < 1e-9);
    }

    #[test]
    fn il_quote_equals_vlp_minus_vhodl() {
        // The core invariant: il_quote should equal V_lp_now − V_hodl_now
        // directly, regardless of how we compute it. Verify by computing
        // both sides independently.
        let (xe, ye) = entry_amounts(100.0, 80.0, 125.0, L);
        let p_now = 110.0;
        let (xl, yl) = entry_amounts(p_now, 80.0, 125.0, L);
        let v_lp_now = xl * p_now + yl;
        let v_hodl_now = xe * p_now + ye;
        let expected = v_lp_now - v_hodl_now;

        let input = PnlInput {
            entry_price: 100.0,
            current_price: p_now,
            lower_price: 80.0,
            upper_price: 125.0,
            entry_x: xe,
            entry_y: ye,
            fees: FeeDelta::ZERO,
        };
        let snap = compute_pnl(input).unwrap();
        let rel = (snap.il_quote - expected).abs() / expected.abs().max(1e-12);
        assert!(
            rel < 1e-9,
            "il_quote={}, expected={}, rel={}",
            snap.il_quote,
            expected,
            rel
        );
    }

    #[test]
    fn il_is_non_positive_and_net_equals_sum() {
        let mut input = base_input();
        input.current_price = 110.0;
        input.fees = FeeDelta {
            base: 0.0,
            quote: 20.0,
        };
        let snap = compute_pnl(input).unwrap();
        assert!(snap.il_quote < 0.0);
        assert!(snap.fees_earned > 0.0);
        assert!((snap.net - (snap.fees_earned + snap.il_quote)).abs() < 1e-9);
    }

    #[test]
    fn fees_can_outpace_il() {
        let mut input = base_input();
        input.current_price = 101.0;
        input.fees = FeeDelta {
            base: 0.0,
            quote: 100.0,
        };
        let snap = compute_pnl(input).unwrap();
        assert!(snap.net > 0.0);
    }

    #[test]
    fn il_can_outpace_fees() {
        let mut input = base_input();
        input.current_price = 200.0;
        let snap = compute_pnl(input).unwrap();
        assert!(snap.net < 0.0);
        assert_eq!(snap.fees_earned, 0.0);
    }

    #[test]
    fn invalid_inputs_error() {
        let mut input = base_input();
        input.entry_x = -1.0;
        assert!(compute_pnl(input).is_err());

        let mut input = base_input();
        input.entry_x = 0.0;
        input.entry_y = 0.0;
        assert!(compute_pnl(input).is_err());

        let mut input = base_input();
        input.fees.base = -1.0;
        assert!(compute_pnl(input).is_err());

        let mut input = base_input();
        input.fees.quote = f64::NAN;
        assert!(compute_pnl(input).is_err());

        let mut input = base_input();
        input.current_price = 0.0;
        assert!(compute_pnl(input).is_err());
    }
}
