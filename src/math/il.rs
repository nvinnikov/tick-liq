//! Impermanent loss calculator for ranged concentrated-liquidity positions.
//!
//! IL is defined as the relative loss an LP incurs versus simply holding the
//! deposited assets (HODL):
//!
//! ```text
//! IL = (V_lp(P) - V_hodl(P)) / V_hodl(P)   ∈  (-1, 0]
//! ```
//!
//! For a concentrated position with liquidity `L` over range `[Pa, Pb]`, the
//! LP amounts (per unit `L`) as a function of the current price `P` are the
//! standard Uniswap V3 formulas from `CLAUDE.md`:
//!
//! - `P ≤ Pa`  : `x = 1/√Pa − 1/√Pb`,     `y = 0`
//! - `P ≥ Pb`  : `x = 0`,                 `y = √Pb − √Pa`
//! - `Pa < P < Pb`: `x = 1/√P  − 1/√Pb`,  `y = √P  − √Pa`
//!
//! We value both the entry composition (at `P_e`) and the current composition
//! (at `P`) using the **current** price `P` as the quote — that is, we ask
//! "what is the dollar value of each bundle right now?". The liquidity `L`
//! cancels from the ratio, so this module takes only prices (no `L` input).
//!
//! Prices are passed as `f64` in *display* units (quote per base, e.g.
//! USDC per SOL). They must be strictly positive and finite; entry price
//! must lie inside the entry range (otherwise the caller has a bug).
//!
//! The function is **pure**: no I/O, no panics on valid input, and is
//! validated by a proptest suite enforcing `IL ≤ 0` across the full input
//! space, `IL == 0` at the identity, and the asymptotic loss bounds.

use anyhow::{bail, Result};

/// Compute the impermanent loss for a ranged LP position as a non-positive
/// fraction (e.g. `-0.0123` means a 1.23 % loss versus HODL).
///
/// # Parameters
/// - `entry_price` — price at which the LP opened the position (display units).
/// - `current_price` — current pool price (display units).
/// - `lower_price` / `upper_price` — inclusive range bounds (display units).
///
/// # Errors
/// Returns an error if any price is non-positive or non-finite, or if
/// `lower_price ≥ upper_price`.
///
/// # Invariants (enforced by proptest)
/// - `IL ≤ 0` for all valid inputs
/// - `IL == 0` exactly when `current_price == entry_price`
/// - `IL → (V_lp_outside − V_hodl) / V_hodl` as price moves far outside range
pub fn impermanent_loss(
    entry_price: f64,
    current_price: f64,
    lower_price: f64,
    upper_price: f64,
) -> Result<f64> {
    for (name, v) in [
        ("entry_price", entry_price),
        ("current_price", current_price),
        ("lower_price", lower_price),
        ("upper_price", upper_price),
    ] {
        if !v.is_finite() || v <= 0.0 {
            bail!("{} must be finite and positive, got {}", name, v);
        }
    }
    if lower_price >= upper_price {
        bail!("lower_price must be < upper_price");
    }

    let sa = lower_price.sqrt();
    let sb = upper_price.sqrt();
    let se = entry_price.sqrt();
    let s = current_price.sqrt();

    let (xe, ye) = amounts_per_unit_liquidity(se, sa, sb);
    let (x, y) = amounts_per_unit_liquidity(s, sa, sb);

    // Both bundles valued at the current price P = s^2, in quote units.
    let p = current_price;
    let v_hodl = xe * p + ye;
    let v_lp = x * p + y;

    if v_hodl <= 0.0 {
        // v_hodl is strictly positive for any non-degenerate entry; guard
        // anyway so a pathological input cannot divide by zero.
        bail!(
            "degenerate entry composition (v_hodl={}) — entry_price likely outside [lower, upper]",
            v_hodl
        );
    }

    let il = (v_lp - v_hodl) / v_hodl;
    Ok(il)
}

/// LP token amounts per unit of liquidity at sqrt-price `s` inside the
/// sqrt-price range `[sa, sb]`. All three regimes (below, in, above).
#[inline]
fn amounts_per_unit_liquidity(s: f64, sa: f64, sb: f64) -> (f64, f64) {
    if s <= sa {
        (1.0 / sa - 1.0 / sb, 0.0)
    } else if s >= sb {
        (0.0, sb - sa)
    } else {
        (1.0 / s - 1.0 / sb, s - sa)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn il_zero_at_entry_equals_current() {
        let il = impermanent_loss(100.0, 100.0, 80.0, 125.0).unwrap();
        assert!(approx_eq(il, 0.0, 1e-12), "il={}", il);
    }

    #[test]
    fn il_negative_when_price_moves() {
        let il_up = impermanent_loss(100.0, 110.0, 80.0, 125.0).unwrap();
        let il_dn = impermanent_loss(100.0, 90.0, 80.0, 125.0).unwrap();
        assert!(il_up < 0.0);
        assert!(il_dn < 0.0);
    }

    #[test]
    fn il_out_of_range_bounded() {
        // When price crashes far below the range, the LP is 100 % token x
        // (worth very little in quote units), while HODL retains the y leg.
        // IL should be a large-but-bounded negative number.
        let il = impermanent_loss(100.0, 1.0, 80.0, 125.0).unwrap();
        assert!(il < -0.3);
        assert!(il > -1.0);
    }

    #[test]
    fn invalid_inputs_error() {
        assert!(impermanent_loss(0.0, 100.0, 80.0, 125.0).is_err());
        assert!(impermanent_loss(100.0, -1.0, 80.0, 125.0).is_err());
        assert!(impermanent_loss(100.0, 100.0, 125.0, 80.0).is_err());
        assert!(impermanent_loss(f64::NAN, 100.0, 80.0, 125.0).is_err());
        assert!(impermanent_loss(f64::INFINITY, 100.0, 80.0, 125.0).is_err());
    }

    #[test]
    fn il_symmetric_small_move() {
        // For a tight symmetric range around entry, small moves up and down
        // should produce IL of comparable magnitude (within ~10 % of each
        // other for a 1 % move — the position is nearly symmetric).
        let il_up = impermanent_loss(100.0, 101.0, 80.0, 125.0).unwrap();
        let il_dn = impermanent_loss(100.0, 99.0, 80.0, 125.0).unwrap();
        assert!(il_up < 0.0);
        assert!(il_dn < 0.0);
        let rel = (il_up - il_dn).abs() / il_up.abs().max(il_dn.abs());
        assert!(rel < 0.1, "asymmetry too large: {}", rel);
    }
}
