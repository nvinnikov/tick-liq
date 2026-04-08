//! Fee accrual math (pure).

/// Compute fees accrued since last on-chain checkpoint (not yet in fee_owed).
///
/// Orca accumulates fees as: fee_growth_per_unit_liquidity * 2^128.
/// Uncollected = (fee_growth_global - fee_growth_checkpoint) * liquidity / 2^128.
pub fn compute_accrued_fees(
    fee_growth_global: u128,
    fee_growth_checkpoint: u128,
    liquidity: u128,
) -> u64 {
    let growth_delta = fee_growth_global.wrapping_sub(fee_growth_checkpoint);
    // We need (growth_delta * liquidity) >> 128.
    // Split each into high and low 64-bit halves to avoid overflow.
    let a_hi = growth_delta >> 64;
    let a_lo = growth_delta & (u64::MAX as u128);
    let b_hi = liquidity >> 64;
    let b_lo = liquidity & (u64::MAX as u128);

    // Full 256-bit product = a_hi*b_hi * 2^128 + (a_hi*b_lo + a_lo*b_hi) * 2^64 + a_lo*b_lo
    // We want bits [128..255], i.e. the upper 128 bits shifted right by 128.
    let hi_hi = a_hi * b_hi;
    let hi_lo = a_hi * b_lo;
    let lo_hi = a_lo * b_hi;
    let lo_lo = a_lo * b_lo;

    // The mid terms contribute their upper 64 bits to the result
    let mid_sum = (hi_lo & (u64::MAX as u128)) + (lo_hi & (u64::MAX as u128)) + (lo_lo >> 64);
    let result = hi_hi + (hi_lo >> 64) + (lo_hi >> 64) + (mid_sum >> 64);
    result as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accrued_fees_zero_when_growth_unchanged() {
        assert_eq!(compute_accrued_fees(1000, 1000, 1_000_000), 0);
    }

    #[test]
    fn test_accrued_fees_increase_with_growth_delta() {
        // fee_growth values are Q128 on-chain, so deltas are large.
        // For (delta * liquidity) >> 128 > 0, we need delta * liquidity > 2^128.
        // Use realistic values: liquidity ~ 2^64, growth_delta ~ 2^64.
        let liquidity: u128 = 1u128 << 64;
        let base: u128 = 0;
        let small = compute_accrued_fees(base + (1u128 << 64), base, liquidity);
        let large = compute_accrued_fees(base + (10u128 << 64), base, liquidity);
        assert!(small > 0, "small should be > 0, got {}", small);
        assert!(large > small, "large={} should be > small={}", large, small);
    }
}
