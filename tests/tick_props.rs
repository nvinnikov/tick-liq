//! Property-based tests for `tick_liq::math::tick`.
//!
//! Invariants enforced (256+ cases each):
//!   1. round-trip: `sqrt_price_x64_to_tick(tick_to_sqrt_price_x64(t)) == t`
//!      (Whirlpool's inverse is a floor, so we assert it matches `t` exactly
//!      because `tick_to_sqrt_price_x64` is the *exact* sqrt-price for the
//!      tick boundary — the floor lands on `t`).
//!   2. monotonicity: `t1 < t2 ⇒ sqrt_price(t1) < sqrt_price(t2)`.
//!   3. spacing alignment: `align_tick_to_spacing(t, s) % s == 0` and the
//!      result is `<= t` (Euclidean floor) and within `s` of `t`.
//!   4. parity with reference: our wrappers agree bit-for-bit with
//!      `orca_whirlpools_core::tick_index_to_sqrt_price`.

use orca_whirlpools_core::tick_index_to_sqrt_price;
use proptest::prelude::*;
use tick_liq::math::tick::{
    align_tick_to_spacing, sqrt_price_x64_to_tick, tick_to_sqrt_price_x64, MAX_TICK, MIN_TICK,
};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn round_trip_tick_sqrt_price_tick(t in MIN_TICK..=MAX_TICK) {
        let sp = tick_to_sqrt_price_x64(t).unwrap();
        let back = sqrt_price_x64_to_tick(sp).unwrap();
        prop_assert_eq!(back, t);
    }

    #[test]
    fn monotonic_in_tick(
        t1 in MIN_TICK..MAX_TICK,
        delta in 1i32..1_000,
    ) {
        let t2 = (t1 as i64 + delta as i64).min(MAX_TICK as i64) as i32;
        prop_assume!(t2 > t1);
        let sp1 = tick_to_sqrt_price_x64(t1).unwrap();
        let sp2 = tick_to_sqrt_price_x64(t2).unwrap();
        prop_assert!(sp1 < sp2);
    }

    #[test]
    fn align_tick_is_multiple_of_spacing(
        t in MIN_TICK..=MAX_TICK,
        s in 1u16..=1024,
    ) {
        let aligned = align_tick_to_spacing(t, s).unwrap();
        prop_assert_eq!(aligned % s as i32, 0);
        prop_assert!(aligned <= t);
        prop_assert!(t - aligned < s as i32);
    }

    #[test]
    fn matches_reference_implementation(t in MIN_TICK..=MAX_TICK) {
        let ours = tick_to_sqrt_price_x64(t).unwrap();
        let theirs = tick_index_to_sqrt_price(t);
        prop_assert_eq!(ours, theirs);
    }
}
