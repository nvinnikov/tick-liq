//! Price-impact math (pure).

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

    PriceImpact {
        target_pct,
        target_price,
        usd_needed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
