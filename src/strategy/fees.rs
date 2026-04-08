//! Fee accrual tracker for LP positions.
//!
//! A [`FeeTracker`] consumes consecutive snapshots of a position's
//! `(fee_growth_inside_0, fee_growth_inside_1, liquidity)` and returns the
//! fees earned between the previous snapshot and the current one, plus a
//! running total.
//!
//! ## Formula
//!
//! On Orca/Raydium CLMM, fee growth inside the position's range is stored as
//! a Q64.128 accumulator (fees-per-unit-liquidity, scaled by `2^128`). Fees
//! earned between two snapshots are:
//!
//! ```text
//! delta_fees = ((fee_growth_inside_now - fee_growth_inside_prev) * liquidity) >> 128
//! ```
//!
//! The subtraction is done with wrapping semantics because the on-chain
//! accumulator is allowed to wrap (it is `u128` and monotonic modulo
//! `2^128`).
//!
//! ## What this tracker does NOT do
//!
//! - It does not read on-chain state; callers supply snapshots.
//! - It does not know about `fee_owed` (already-collected-but-unclaimed
//!   fees). Pair this with a collector that zeroes the checkpoint on
//!   collect — after a collect, feed the next snapshot and the delta will
//!   correctly reflect the post-collect window.
//! - It does not price the fees; pricing is done by passing the current
//!   token prices into [`FeesEarned::with_prices`].

/// One observation of a position's fee-growth state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeSnapshot {
    /// Fee-growth-inside accumulator for token A (Q64.128).
    pub fee_growth_inside_a: u128,
    /// Fee-growth-inside accumulator for token B (Q64.128).
    pub fee_growth_inside_b: u128,
    /// Position liquidity at the time of the snapshot.
    pub liquidity: u128,
}

/// Fees earned between two snapshots, in raw token units (not decimals-adjusted).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FeesEarned {
    /// Token A fees in base units (e.g. lamports for SOL).
    pub token_a: u64,
    /// Token B fees in base units.
    pub token_b: u64,
}

impl FeesEarned {
    /// Convert to a USD value given per-token unit prices. `price_*` is the
    /// USD price of one base unit of the respective token (i.e. already
    /// scaled for decimals by the caller).
    pub fn usd_value(&self, price_a: f64, price_b: f64) -> f64 {
        self.token_a as f64 * price_a + self.token_b as f64 * price_b
    }

    /// Build a `(fees, usd)` tuple; convenience for callers that want both
    /// out of one call.
    pub fn with_prices(self, price_a: f64, price_b: f64) -> (FeesEarned, f64) {
        let usd = self.usd_value(price_a, price_b);
        (self, usd)
    }
}

/// Tracks cumulative fees across a sequence of snapshots.
#[derive(Debug, Default, Clone)]
pub struct FeeTracker {
    last: Option<FeeSnapshot>,
    total: FeesEarned,
}

impl FeeTracker {
    /// Create a tracker with no prior snapshot. The first call to
    /// [`FeeTracker::update`] will return zero fees (there is no previous
    /// point to diff against) and seed the baseline.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a tracker pre-seeded with a baseline snapshot. Useful when
    /// resuming from a persisted checkpoint.
    pub fn with_baseline(snapshot: FeeSnapshot) -> Self {
        Self {
            last: Some(snapshot),
            total: FeesEarned::default(),
        }
    }

    /// Feed a new snapshot. Returns the fees earned *since the previous
    /// snapshot*. The first call (or the first after [`FeeTracker::reset`])
    /// returns zero.
    pub fn update(&mut self, snapshot: FeeSnapshot) -> FeesEarned {
        let delta = match self.last {
            None => FeesEarned::default(),
            Some(prev) => {
                // Use the prior snapshot's liquidity: fees accrued over the
                // interval are proportional to the liquidity that was active
                // during the interval, not the new liquidity.
                let token_a = mul_shift_128(
                    snapshot
                        .fee_growth_inside_a
                        .wrapping_sub(prev.fee_growth_inside_a),
                    prev.liquidity,
                );
                let token_b = mul_shift_128(
                    snapshot
                        .fee_growth_inside_b
                        .wrapping_sub(prev.fee_growth_inside_b),
                    prev.liquidity,
                );
                FeesEarned { token_a, token_b }
            }
        };

        self.total = FeesEarned {
            token_a: self.total.token_a.saturating_add(delta.token_a),
            token_b: self.total.token_b.saturating_add(delta.token_b),
        };
        self.last = Some(snapshot);
        delta
    }

    /// Cumulative fees observed across all snapshots since tracker
    /// construction or the last [`FeeTracker::reset`].
    pub fn total(&self) -> FeesEarned {
        self.total
    }

    /// Most recent snapshot, if any.
    pub fn last_snapshot(&self) -> Option<FeeSnapshot> {
        self.last
    }

    /// Reset cumulative totals and baseline. Call this after the execution
    /// layer has collected fees on-chain — the next snapshot will then
    /// serve as a fresh baseline.
    pub fn reset(&mut self) {
        self.last = None;
        self.total = FeesEarned::default();
    }
}

/// Compute `(growth_delta * liquidity) >> 128`, saturating at `u64::MAX`.
///
/// Done in 256-bit arithmetic by splitting each operand into two `u64`
/// halves; we want bits `[128..192]` of the full 256-bit product.
fn mul_shift_128(growth_delta: u128, liquidity: u128) -> u64 {
    let a_hi = growth_delta >> 64;
    let a_lo = growth_delta & (u64::MAX as u128);
    let b_hi = liquidity >> 64;
    let b_lo = liquidity & (u64::MAX as u128);

    // Full product terms, each fits in u128:
    // full = hi_hi*2^128 + (hi_lo + lo_hi)*2^64 + lo_lo
    let lo_lo = a_lo * b_lo;
    let hi_lo = a_hi * b_lo;
    let lo_hi = a_lo * b_hi;
    let hi_hi = a_hi * b_hi;

    // Sum middle column into bits [64..192]:
    // carry-aware add of the high halves of lo_lo with low halves of hi_lo+lo_hi,
    // promoted past bit 128.
    let mid = (lo_lo >> 64) + (hi_lo & (u64::MAX as u128)) + (lo_hi & (u64::MAX as u128));
    // Bits [128..256] of the full product. This u128 addition is provably
    // non-overflowing: each operand fits in u128 by construction
    // (`hi_hi <= (2^64 - 1)^2 < 2^128 - 2^65 + 1`, the two `>> 64` shifts
    // each fit in u64, and `mid >> 64` fits in u66). The worst case is
    // `growth_delta = liquidity = u128::MAX`, where the closed-form
    // 256-bit product is `(2^128 - 1)^2 = 2^256 - 2^129 + 1`, whose top
    // 128 bits equal `2^128 - 2`, comfortably inside u128. The
    // `mul_shift_128_saturates_on_overflow` test plus the `cross_check`
    // proptest below pin this corner empirically.
    let high = hi_hi + (hi_lo >> 64) + (lo_hi >> 64) + (mid >> 64);
    // We want bits [128..192], i.e. the low 64 bits of `high`. If anything
    // sits in bits [192..256] the result overflowed a u64 and we saturate.
    if high >> 64 != 0 {
        u64::MAX
    } else {
        high as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(a: u128, b: u128, l: u128) -> FeeSnapshot {
        FeeSnapshot {
            fee_growth_inside_a: a,
            fee_growth_inside_b: b,
            liquidity: l,
        }
    }

    #[test]
    fn first_update_returns_zero() {
        let mut t = FeeTracker::new();
        let earned = t.update(snap(1000, 2000, 1_000_000));
        assert_eq!(earned, FeesEarned::default());
        assert_eq!(t.total(), FeesEarned::default());
    }

    #[test]
    fn flat_growth_yields_zero() {
        let mut t = FeeTracker::with_baseline(snap(42, 42, 1_000_000));
        let earned = t.update(snap(42, 42, 1_000_000));
        assert_eq!(earned, FeesEarned::default());
    }

    #[test]
    fn growth_delta_scaled_by_liquidity() {
        // ((2^127) * L) >> 128 = L / 2
        let base: u128 = 1_000_000;
        let big_growth: u128 = 1u128 << 127;
        let mut t = FeeTracker::with_baseline(snap(0, 0, base));
        let earned = t.update(snap(big_growth, 0, base));
        assert_eq!(earned.token_a, 500_000);
        assert_eq!(earned.token_b, 0);
    }

    #[test]
    fn accumulates_across_multiple_updates() {
        let l: u128 = 2_000_000;
        // Each step advances growth_a by 2^127 (→ l/2 fees per step).
        let step = 1u128 << 127;
        let mut t = FeeTracker::with_baseline(snap(0, 0, l));
        let d1 = t.update(snap(step, 0, l));
        let d2 = t.update(snap(step.wrapping_mul(2) % (u128::MAX), 0, l));
        // Second snapshot is step*2, so delta from first to second is `step`.
        // (step.wrapping_mul(2) may wrap; that's fine because we use wrapping_sub.)
        assert_eq!(d1.token_a, 1_000_000);
        assert_eq!(d2.token_a, 1_000_000);
        assert_eq!(t.total().token_a, 2_000_000);
    }

    #[test]
    fn wrapping_subtraction_handles_accumulator_wrap() {
        // prev near u128::MAX, now just past zero — wrap-safe delta.
        let prev_growth = u128::MAX - (1u128 << 127) + 1; // MAX - 2^127 + 1
        let now_growth: u128 = 1u128 << 127; // wraps to effectively 2^127 past prev? not quite
                                             // Compute expected delta directly:
        let expected_delta = now_growth.wrapping_sub(prev_growth);
        let l: u128 = 1_000_000;
        let mut t = FeeTracker::with_baseline(snap(prev_growth, 0, l));
        let earned = t.update(snap(now_growth, 0, l));
        let expected = mul_shift_128(expected_delta, l);
        assert_eq!(earned.token_a, expected);
    }

    #[test]
    fn uses_prior_liquidity_for_delta() {
        // Fees accrued over the interval should scale by the liquidity that
        // was active DURING the interval (= prev.liquidity), not the new one.
        let prev_l: u128 = 1_000_000;
        let new_l: u128 = 9_999_999; // wildly different — if used, test fails
        let step = 1u128 << 127;
        let mut t = FeeTracker::with_baseline(snap(0, 0, prev_l));
        let earned = t.update(snap(step, 0, new_l));
        assert_eq!(earned.token_a as u128, prev_l / 2);
    }

    #[test]
    fn reset_clears_baseline_and_total() {
        let mut t = FeeTracker::with_baseline(snap(0, 0, 1_000_000));
        t.update(snap(1u128 << 127, 0, 1_000_000));
        assert!(t.total().token_a > 0);
        t.reset();
        assert_eq!(t.total(), FeesEarned::default());
        assert!(t.last_snapshot().is_none());
        // First update after reset is zero (re-baselining).
        let earned = t.update(snap(1u128 << 127, 0, 1_000_000));
        assert_eq!(earned, FeesEarned::default());
    }

    #[test]
    fn usd_value_multiplies_per_token_prices() {
        let fees = FeesEarned {
            token_a: 1_500_000_000, // 1.5 SOL in lamports
            token_b: 2_500_000,     // 2.5 USDC in 6-dp units
        };
        // Price per BASE unit: $100 / 1e9 per lamport, $1 / 1e6 per micro-USDC.
        let usd = fees.usd_value(100.0 / 1e9, 1.0 / 1e6);
        assert!((usd - (150.0 + 2.5)).abs() < 1e-9);
    }

    #[test]
    fn mul_shift_128_saturates_on_overflow() {
        // Both operands max: result would be ~2^128 which exceeds u64.
        let out = mul_shift_128(u128::MAX, u128::MAX);
        assert_eq!(out, u64::MAX);
    }

    // Reference implementation: full 256-bit product via ethnum, shifted
    // right by 128 and clamped to u64. Used by the proptest below to pin
    // `mul_shift_128` to its mathematical definition across the input space.
    fn reference_mul_shift_128(a: u128, b: u128) -> u64 {
        use ethnum::U256;
        let prod = U256::from(a) * U256::from(b);
        let shifted = prod >> 128u32;
        if shifted > U256::from(u64::MAX) {
            u64::MAX
        } else {
            shifted.as_u64()
        }
    }

    proptest::proptest! {
        #[test]
        fn mul_shift_128_matches_reference(a: u128, b: u128) {
            proptest::prop_assert_eq!(mul_shift_128(a, b), reference_mul_shift_128(a, b));
        }
    }

    #[test]
    fn both_tokens_track_independently() {
        let l: u128 = 1_000_000;
        let mut t = FeeTracker::with_baseline(snap(0, 0, l));
        let a_step = 1u128 << 127; // -> l/2
        let b_step = 1u128 << 126; // -> l/4
        let earned = t.update(snap(a_step, b_step, l));
        assert_eq!(earned.token_a, 500_000);
        assert_eq!(earned.token_b, 250_000);
    }
}
