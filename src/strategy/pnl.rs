//! Real P&L engine: fees earned minus impermanent loss.
//!
//! Composes [`crate::math::il::impermanent_loss`] with an accumulated-fee
//! input to produce a single `net` figure (in quote units) that represents
//! what the LP has actually made versus a HODL baseline.
//!
//! ```text
//! net = fees_earned_quote + il_quote
//! ```
//!
//! where `il_quote` is a **non-positive** quote-denominated loss (IL in
//! fraction terms, multiplied by the HODL value at the current price) and
//! `fees_earned_quote` is the sum of all fee claims since entry, both legs
//! converted to quote at the current price.
//!
//! Sign convention: `il_quote ≤ 0`, `fees_earned_quote ≥ 0`, `net` can be
//! either sign — it is strictly positive exactly when fees have outpaced IL.
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
    /// Impermanent loss in quote units (non-positive). This is the IL
    /// *fraction* from [`crate::math::il::impermanent_loss`] multiplied by
    /// the HODL value of the initial deposit at the current price.
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
    /// Initial deposit valued at entry, in quote units. This is the
    /// denominator for the IL fraction → absolute conversion: IL is
    /// `fraction * deposit_value_at_entry`, which is the correct HODL
    /// baseline to compare fees against.
    pub deposit_quote_at_entry: f64,
    /// Accumulated fees earned since entry.
    pub fees: FeeDelta,
}

/// Compute a P&L snapshot from the given inputs.
///
/// # Errors
/// Returns an error on any non-finite / non-positive price, inverted range,
/// negative or non-finite fee amounts, or non-positive deposit.
pub fn compute_pnl(input: PnlInput) -> Result<PnlSnapshot> {
    let PnlInput {
        entry_price,
        current_price,
        lower_price,
        upper_price,
        deposit_quote_at_entry,
        fees,
    } = input;

    if !deposit_quote_at_entry.is_finite() || deposit_quote_at_entry <= 0.0 {
        bail!(
            "deposit_quote_at_entry must be finite and positive, got {}",
            deposit_quote_at_entry
        );
    }
    for (name, v) in [("fees.base", fees.base), ("fees.quote", fees.quote)] {
        if !v.is_finite() || v < 0.0 {
            bail!("{} must be finite and non-negative, got {}", name, v);
        }
    }

    // Delegates all price validation to impermanent_loss.
    let il_fraction = impermanent_loss(entry_price, current_price, lower_price, upper_price)?;

    let fees_earned = fees.to_quote(current_price);
    let il_quote = il_fraction * deposit_quote_at_entry;
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

    fn base_input() -> PnlInput {
        PnlInput {
            entry_price: 100.0,
            current_price: 100.0,
            lower_price: 80.0,
            upper_price: 125.0,
            deposit_quote_at_entry: 10_000.0,
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
        // Small price move + big fees → net positive.
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
        // Large price move + no fees → net strongly negative.
        let mut input = base_input();
        input.current_price = 200.0;
        let snap = compute_pnl(input).unwrap();
        assert!(snap.net < 0.0);
        assert_eq!(snap.fees_earned, 0.0);
    }

    #[test]
    fn invalid_inputs_error() {
        let mut input = base_input();
        input.deposit_quote_at_entry = 0.0;
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
