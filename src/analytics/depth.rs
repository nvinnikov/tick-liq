/// One price level with its total active liquidity.
#[derive(Debug, Clone)]
pub struct LiquidityLevel {
    pub price: f64,
    pub liquidity: u128,
}

/// Estimated price impact for a trade.
#[derive(Debug, Clone)]
pub struct PriceImpact {
    pub target_pct: f64,
    pub target_price: f64,
    pub usd_needed: f64,
}

/// Build a bucketed liquidity distribution around the current price.
///
/// tick_liquidities: (tick_index, net_liquidity_delta) pairs from tick array accounts.
/// Uses the net-liquidity-delta model: liquidity at a tick = sum of all deltas at or below it.
/// For now, accepts an empty slice — pool-level liquidity is used as a fallback in callers.
pub fn build_distribution(
    tick_liquidities: &[(i32, i64)],
    current_tick: i32,
    tick_spacing: i32,
    n_buckets_each_side: usize,
) -> Vec<LiquidityLevel> {
    if tick_liquidities.is_empty() {
        return vec![];
    }

    let mut result = Vec::with_capacity(n_buckets_each_side * 2 + 1);
    let mut running: i128 = 0;

    for i in 0..=(n_buckets_each_side * 2) {
        let bucket_start = current_tick
            - (n_buckets_each_side as i32) * tick_spacing
            + i as i32 * tick_spacing;
        let bucket_end = bucket_start + tick_spacing;

        let delta: i64 = tick_liquidities
            .iter()
            .filter(|(t, _)| *t >= bucket_start && *t < bucket_end)
            .map(|(_, d)| *d)
            .sum();

        running += delta as i128;

        let mid_tick = bucket_start + tick_spacing / 2;
        let price = 1.0001f64.powi(mid_tick);

        result.push(LiquidityLevel {
            price,
            liquidity: running.unsigned_abs(),
        });
    }

    result
}

/// Estimate USD trade size needed to move price by target_pct%.
///
/// Uses the CLMM constant-liquidity approximation:
///   buy token A:  amount_a = L * (1/sqrt(P) - 1/sqrt(P_target))
///   USD cost = amount_a * P_current
pub fn estimate_impact(
    current_price: f64,
    liquidity: u128,
    target_pct: f64,
    is_buy: bool,
) -> PriceImpact {
    let l = liquidity as f64;
    let target_price = if is_buy {
        current_price * (1.0 + target_pct / 100.0)
    } else {
        current_price * (1.0 - target_pct / 100.0)
    };

    let sqrt_p = current_price.sqrt();
    let sqrt_target = target_price.sqrt();
    let amount_a = l * (1.0 / sqrt_p - 1.0 / sqrt_target).abs();
    let usd_needed = amount_a * current_price;

    PriceImpact { target_pct, target_price, usd_needed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_ticks_returns_empty() {
        let result = build_distribution(&[], 0, 64, 4);
        assert!(result.is_empty());
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

    #[test]
    fn test_target_price_correct_direction_for_buy() {
        let impact = estimate_impact(100.0, 1_000_000, 2.0, true);
        assert!(impact.target_price > 100.0);
    }

    #[test]
    fn test_target_price_correct_direction_for_sell() {
        let impact = estimate_impact(100.0, 1_000_000, 2.0, false);
        assert!(impact.target_price < 100.0);
    }
}
