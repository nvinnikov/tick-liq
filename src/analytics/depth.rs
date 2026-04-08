//! Liquidity depth analysis. Pure math lives in `crate::math::impact`.

pub use crate::math::impact::{estimate_impact, LiquidityLevel};

/// Build a bucketed liquidity distribution around the current tick from real
/// on-chain TickArray data.
///
/// `tick_liquidities`: `(tick_index, liquidity_net)` pairs from TickArray
/// accounts. Only initialized ticks need be included.
///
/// `current_liquidity` is the pool's active liquidity at `current_tick` (from
/// the Whirlpool account). It anchors the prefix sum so ticks outside the
/// sampled window don't distort the running total.
///
/// Algorithm: walk left from `current_tick` subtracting deltas (crossing a
/// tick downward removes its `liquidity_net`), walk right adding deltas
/// (crossing upward adds `liquidity_net`). Each bucket's reported liquidity is
/// the active L mid-bucket.
pub fn build_distribution(
    tick_liquidities: &[(i32, i128)],
    current_liquidity: u128,
    current_tick: i32,
    tick_spacing: i32,
    n_buckets_each_side: usize,
) -> Vec<LiquidityLevel> {
    let total = n_buckets_each_side * 2 + 1;
    let mut result: Vec<LiquidityLevel> = Vec::with_capacity(total);

    // Right-walk (ascending ticks): add liquidity_net when crossing upward.
    let mut running_right: i128 = current_liquidity as i128;
    for i in 0..=(n_buckets_each_side as i32) {
        let bucket_start = current_tick + i * tick_spacing;
        let bucket_end = bucket_start + tick_spacing;
        if i > 0 {
            let prev_start = bucket_start - tick_spacing;
            let delta: i128 = tick_liquidities
                .iter()
                .filter(|(t, _)| *t > prev_start && *t <= bucket_start)
                .map(|(_, d)| *d)
                .sum();
            running_right += delta;
        }
        let mid = bucket_start + tick_spacing / 2;
        let price = 1.0001f64.powi(mid);
        result.push(LiquidityLevel {
            price,
            liquidity: running_right.max(0) as u128,
        });
        let _ = bucket_end;
    }

    // Left-walk (descending): subtract liquidity_net when crossing downward.
    let mut running_left: i128 = current_liquidity as i128;
    let mut left: Vec<LiquidityLevel> = Vec::with_capacity(n_buckets_each_side);
    for i in 1..=(n_buckets_each_side as i32) {
        let bucket_start = current_tick - i * tick_spacing;
        let upper = bucket_start + tick_spacing;
        let delta: i128 = tick_liquidities
            .iter()
            .filter(|(t, _)| *t > bucket_start && *t <= upper)
            .map(|(_, d)| *d)
            .sum();
        running_left -= delta;
        let mid = bucket_start + tick_spacing / 2;
        let price = 1.0001f64.powi(mid);
        left.push(LiquidityLevel {
            price,
            liquidity: running_left.max(0) as u128,
        });
    }

    // Assemble [left reversed] + [right].
    left.reverse();
    left.extend(result);
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_ticks_uses_current_liquidity_flat() {
        let result = build_distribution(&[], 1_000_000, 0, 64, 4);
        assert_eq!(result.len(), 9);
        for level in &result {
            assert_eq!(level.liquidity, 1_000_000);
        }
    }

    #[test]
    fn test_synthetic_ticks_prefix_sum() {
        // Pool at tick 0, spacing 64, current L = 1000.
        // Tick +64 adds 500 (upward cross), tick -64 adds 200 when crossed upward,
        // so crossing it downward from 0 should remove 200.
        let ticks: Vec<(i32, i128)> = vec![(64, 500), (-64, 200)];
        let result = build_distribution(&ticks, 1000, 0, 64, 2);
        assert_eq!(result.len(), 5);
        // Bucket 0 (current): 1000
        assert_eq!(result[2].liquidity, 1000);
        // Bucket +1 (ticks 64..128): crossed +64 upward -> 1000 + 500 = 1500
        assert_eq!(result[3].liquidity, 1500);
        // Bucket -1 (ticks -64..0): tick -64 not yet crossed downward -> still 1000
        assert_eq!(result[1].liquidity, 1000);
        // Downward cross further out: add another tick and check with 2 buckets each side.
        let ticks2: Vec<(i32, i128)> = vec![(-64, 200)];
        let r2 = build_distribution(&ticks2, 1000, 0, 64, 2);
        // Bucket -2 (ticks -128..-64): crossed -64 downward -> 1000 - 200 = 800
        assert_eq!(r2[0].liquidity, 800);
    }

    #[test]
    fn test_higher_liquidity_needs_larger_trade_for_same_impact() {
        let small = estimate_impact(100.0, 1_000, 1.0, true);
        let large = estimate_impact(100.0, 1_000_000_000, 1.0, true);
        assert!(large.usd_needed > small.usd_needed);
    }

    #[test]
    fn test_larger_pct_needs_more_usd() {
        let one_pct = estimate_impact(100.0, 1_000_000, 1.0, true);
        let five_pct = estimate_impact(100.0, 1_000_000, 5.0, true);
        assert!(five_pct.usd_needed > one_pct.usd_needed);
    }
}
