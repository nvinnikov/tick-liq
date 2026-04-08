//! Property-based tests for `tick_liq::strategy::pnl`.
//!
//! Invariants:
//!   1. fees_earned ≥ 0
//!   2. il_quote ≤ 0
//!   3. net == fees_earned + il_quote (bit-identity within float tolerance)
//!   4. Zero fees + entry == current ⇒ snapshot is all zeros
//!   5. net scales linearly in fee amounts (holding prices fixed)

use proptest::prelude::*;
use tick_liq::strategy::pnl::{compute_pnl, FeeDelta, PnlInput};

fn input_strategy() -> impl Strategy<Value = PnlInput> {
    (
        10f64..10_000.0,   // entry_price
        0.1f64..10.0,      // current/entry ratio
        0.1f64..0.99,      // lower_frac
        1.01f64..10.0,     // upper_frac
        1f64..1_000_000.0, // deposit
        0f64..1_000.0,     // fees.base
        0f64..1_000_000.0, // fees.quote
    )
        .prop_map(
            |(entry, ratio, lower_frac, upper_frac, deposit, fb, fq)| PnlInput {
                entry_price: entry,
                current_price: entry * ratio,
                lower_price: entry * lower_frac,
                upper_price: entry * upper_frac,
                deposit_quote_at_entry: deposit,
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
    fn fees_non_negative_il_non_positive_and_net_consistent(input in input_strategy()) {
        let snap = compute_pnl(input).unwrap();
        prop_assert!(snap.fees_earned >= 0.0);
        prop_assert!(snap.il_quote <= 1e-12); // allow tiny float noise on the zero boundary
        let drift = (snap.net - (snap.fees_earned + snap.il_quote)).abs();
        prop_assert!(drift < 1e-9 * snap.fees_earned.abs().max(snap.il_quote.abs()).max(1.0));
    }

    #[test]
    fn zero_when_no_move_and_no_fees(
        entry in 10f64..10_000.0,
        lower_frac in 0.1f64..0.99,
        upper_frac in 1.01f64..10.0,
        deposit in 1f64..1_000_000.0,
    ) {
        let input = PnlInput {
            entry_price: entry,
            current_price: entry,
            lower_price: entry * lower_frac,
            upper_price: entry * upper_frac,
            deposit_quote_at_entry: deposit,
            fees: FeeDelta::ZERO,
        };
        let snap = compute_pnl(input).unwrap();
        prop_assert!(snap.fees_earned == 0.0);
        prop_assert!(snap.il_quote.abs() < 1e-9 * deposit);
        prop_assert!(snap.net.abs() < 1e-9 * deposit);
    }

    #[test]
    fn fees_scale_linearly(
        base_input in input_strategy(),
        scale in 2f64..100.0,
    ) {
        let snap1 = compute_pnl(base_input).unwrap();
        let scaled = PnlInput {
            fees: FeeDelta {
                base: base_input.fees.base * scale,
                quote: base_input.fees.quote * scale,
            },
            ..base_input
        };
        let snap2 = compute_pnl(scaled).unwrap();
        // IL component is unchanged, fee component is scaled.
        prop_assert!((snap2.il_quote - snap1.il_quote).abs() < 1e-9 * snap1.il_quote.abs().max(1.0));
        let target_fees = snap1.fees_earned * scale;
        let drift = (snap2.fees_earned - target_fees).abs();
        prop_assert!(drift < 1e-9 * target_fees.max(1.0));
    }
}
