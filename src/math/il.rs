//! Impermanent loss and P&L aggregation (pure math).

/// P&L result in USD (token B equivalent).
#[derive(Debug, Clone)]
pub struct PnlResult {
    pub fees_usd: f64,
    pub il_usd: f64, // always <= 0
    pub net_usd: f64,
    pub initial_value_usd: f64,
}

impl PnlResult {
    pub fn fees_pct(&self) -> f64 {
        if self.initial_value_usd == 0.0 {
            return 0.0;
        }
        self.fees_usd / self.initial_value_usd * 100.0
    }

    pub fn il_pct(&self) -> f64 {
        if self.initial_value_usd == 0.0 {
            return 0.0;
        }
        self.il_usd / self.initial_value_usd * 100.0
    }

    pub fn net_pct(&self) -> f64 {
        if self.initial_value_usd == 0.0 {
            return 0.0;
        }
        self.net_usd / self.initial_value_usd * 100.0
    }
}

/// Compute impermanent loss as a fraction (e.g. -0.02 = -2%).
///
/// Uses the standard concentrated liquidity IL formula.
/// Clamps prices to range boundaries before computing.
/// Returns 0.0 if entry price is 0 (unknown).
pub fn compute_il(price_entry: f64, price_current: f64, price_lower: f64, price_upper: f64) -> f64 {
    if price_entry == 0.0 {
        return 0.0;
    }

    let pa = price_lower.sqrt();
    let pb = price_upper.sqrt();
    let sp0 = price_entry.sqrt().clamp(pa, pb);
    let sp1 = price_current.sqrt().clamp(pa, pb);

    let ratio = sp1 / sp0;
    // V_lp / V_hodl = 2*sqrt(ratio) / (1 + ratio)
    let lp_relative = 2.0 * ratio.sqrt() / (1.0 + ratio);

    lp_relative - 1.0 // always <= 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_il_zero_at_entry_price() {
        let il = compute_il(100.0, 100.0, 80.0, 120.0);
        assert!(
            il.abs() < 1e-10,
            "IL at entry price should be ~0, got {}",
            il
        );
    }

    #[test]
    fn test_il_always_non_positive() {
        for price in [50.0, 80.0, 90.0, 100.0, 110.0, 130.0, 200.0] {
            let il = compute_il(100.0, price, 80.0, 120.0);
            assert!(il <= 0.0, "IL must be <= 0 for price {}, got {}", price, il);
        }
    }

    #[test]
    fn test_il_zero_when_entry_unknown() {
        assert_eq!(compute_il(0.0, 150.0, 80.0, 120.0), 0.0);
    }
}
