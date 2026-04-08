//! Property-based tests for `tick_liq::strategy::pnl`.
//!
//! Invariants:
//!   1. fees_earned ≥ 0, il_quote ≤ 0, net == fees_earned + il_quote
//!   2. il_quote == V_lp_now − V_hodl_now (the core semantic anchor — this
//!      is the invariant that would catch entry-anchored IL misuse)
//!   3. Zero fees + entry == current ⇒ snapshot is all zeros
//!   4. net scales linearly in fee amounts (holding prices fixed)

use proptest::prelude::*;
use tick_liq::strategy::pnl::{compute_pnl, FeeDelta, PnlInput};

/// Uniswap V3 position amounts at price `p` for liquidity `l` in range
/// `[lower, upper]` — three regimes.
fn amounts(p: f64, lower: f64, upper: f64, l: f64) -> (f64, f64) {
    let sa = lower.sqrt();
    let sb = upper.sqrt();
    let s = p.sqrt();
    if s <= sa {
        (l * (1.0 / sa - 1.0 / sb), 0.0)
    } else if s >= sb {
        (0.0, l * (sb - sa))
    } else {
        (l * (1.0 / s - 1.0 / sb), l * (s - sa))
    }
}

#[derive(Debug, Clone, Copy)]
struct Scenario {
    entry_price: f64,
    current_price: f64,
    lower_price: f64,
    upper_price: f64,
    l: f64,
    fees: FeeDelta,
}

impl Scenario {
    fn input(&self) -> PnlInput {
        let (xe, ye) = amounts(self.entry_price, self.lower_price, self.upper_price, self.l);
        PnlInput {
            entry_price: self.entry_price,
            current_price: self.current_price,
            lower_price: self.lower_price,
            upper_price: self.upper_price,
            entry_x: xe,
            entry_y: ye,
            fees: self.fees,
        }
    }
}

fn scenario_strategy() -> impl Strategy<Value = Scenario> {
    (
        10f64..10_000.0,   // entry_price
        0.1f64..10.0,      // current/entry ratio
        0.1f64..0.99,      // lower_frac
        1.01f64..10.0,     // upper_frac
        1f64..1_000_000.0, // liquidity
        0f64..1_000.0,     // fees.base
        0f64..1_000_000.0, // fees.quote
    )
        .prop_map(
            |(entry, ratio, lower_frac, upper_frac, l, fb, fq)| Scenario {
                entry_price: entry,
                current_price: entry * ratio,
                lower_price: entry * lower_frac,
                upper_price: entry * upper_frac,
                l,
                fees: FeeDelta {
                    base: fb,
                    quote: fq,
                },
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn signs_and_net_consistent(scn in scenario_strategy()) {
        let snap = compute_pnl(scn.input()).unwrap();
        prop_assert!(snap.fees_earned >= 0.0);
        prop_assert!(snap.il_quote <= 1e-9 * snap.il_quote.abs().max(1.0));
        let drift = (snap.net - (snap.fees_earned + snap.il_quote)).abs();
        prop_assert!(drift < 1e-9 * snap.fees_earned.abs().max(snap.il_quote.abs()).max(1.0));
    }

    #[test]
    fn il_quote_equals_vlp_minus_vhodl(scn in scenario_strategy()) {
        // Independently compute V_lp_now and V_hodl_now, and verify
        // il_quote matches V_lp_now − V_hodl_now. This is the invariant
        // that pins the present-value anchoring.
        let snap = compute_pnl(scn.input()).unwrap();
        let (xe, ye) = amounts(scn.entry_price, scn.lower_price, scn.upper_price, scn.l);
        let (xl, yl) = amounts(scn.current_price, scn.lower_price, scn.upper_price, scn.l);
        let v_hodl_now = xe * scn.current_price + ye;
        let v_lp_now = xl * scn.current_price + yl;
        let expected = v_lp_now - v_hodl_now;
        let drift = (snap.il_quote - expected).abs();
        let scale = expected.abs().max(v_hodl_now).max(1.0);
        prop_assert!(
            drift < 1e-9 * scale,
            "il_quote={}, expected={}, drift={}, scale={}",
            snap.il_quote,
            expected,
            drift,
            scale
        );
    }

    #[test]
    fn zero_when_no_move_and_no_fees(
        entry in 10f64..10_000.0,
        lower_frac in 0.1f64..0.99,
        upper_frac in 1.01f64..10.0,
        l in 1f64..1_000_000.0,
    ) {
        let lower = entry * lower_frac;
        let upper = entry * upper_frac;
        let (xe, ye) = amounts(entry, lower, upper, l);
        let input = PnlInput {
            entry_price: entry,
            current_price: entry,
            lower_price: lower,
            upper_price: upper,
            entry_x: xe,
            entry_y: ye,
            fees: FeeDelta::ZERO,
        };
        let snap = compute_pnl(input).unwrap();
        let v_hodl = xe * entry + ye;
        prop_assert!(snap.fees_earned == 0.0);
        prop_assert!(snap.il_quote.abs() < 1e-9 * v_hodl.max(1.0));
        prop_assert!(snap.net.abs() < 1e-9 * v_hodl.max(1.0));
    }

    #[test]
    fn fees_scale_linearly(
        scn in scenario_strategy(),
        scale in 2f64..100.0,
    ) {
        let snap1 = compute_pnl(scn.input()).unwrap();
        let mut scn2 = scn;
        scn2.fees = FeeDelta {
            base: scn.fees.base * scale,
            quote: scn.fees.quote * scale,
        };
        let snap2 = compute_pnl(scn2.input()).unwrap();
        prop_assert!((snap2.il_quote - snap1.il_quote).abs() < 1e-9 * snap1.il_quote.abs().max(1.0));
        let target_fees = snap1.fees_earned * scale;
        let drift = (snap2.fees_earned - target_fees).abs();
        prop_assert!(drift < 1e-9 * target_fees.max(1.0));
    }
}
