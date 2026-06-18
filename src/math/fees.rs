//! Fee accrual math (pure).

/// Compute fees accrued since last on-chain checkpoint (not yet in fee_owed).
///
/// Whirlpool stores fee_growth_global as Q64.64 (scaled by 2^64):
///   fee_growth_global += fee_amount * 2^64 / total_in_range_liquidity
///
/// Uncollected = (fee_growth_delta * liquidity) >> 64.
pub fn compute_accrued_fees(
    fee_growth_global: u128,
    fee_growth_checkpoint: u128,
    liquidity: u128,
) -> u64 {
    let growth_delta = fee_growth_global.wrapping_sub(fee_growth_checkpoint);
    // We need (growth_delta * liquidity) >> 64.
    // Split into 64-bit halves to stay within u128.
    let a_hi = growth_delta >> 64;
    let a_lo = growth_delta & (u64::MAX as u128);
    let b_hi = liquidity >> 64;
    let b_lo = liquidity & (u64::MAX as u128);

    // (a * b) >> 64 = a_hi*b_hi * 2^64 + (a_hi*b_lo + a_lo*b_hi) + (a_lo*b_lo >> 64)
    // Lower 64 bits only (the a_hi*b_hi * 2^64 term is negligible for realistic LP values):
    let mid = (a_hi * b_lo)
        .wrapping_add(a_lo * b_hi)
        .wrapping_add((a_lo * b_lo) >> 64);
    mid as u64
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
        // Whirlpool uses Q64.64: fee_growth_delta * liquidity >> 64.
        // growth_delta=1<<64 with liquidity=1 → 1 token unit.
        let liquidity: u128 = 1_000_000; // realistic small position
        let base: u128 = 0;
        // growth_delta = 1<<64 means exactly 1 fee token per unit liquidity
        let one_token_per_unit = 1u128 << 64;
        let result = compute_accrued_fees(base + one_token_per_unit, base, liquidity);
        assert_eq!(result, liquidity as u64, "should equal liquidity tokens");

        let small = compute_accrued_fees(base + one_token_per_unit, base, liquidity);
        let large = compute_accrued_fees(base + 10 * one_token_per_unit, base, liquidity);
        assert!(small > 0, "small should be > 0, got {}", small);
        assert!(large > small, "large={} should be > small={}", large, small);
    }
}
