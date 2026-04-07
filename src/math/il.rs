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

    // Standard CLMM IL: V_LP/V_HODL = 2√k/(1+k) where k = P1/P0.
    // In sqrt-price terms: = 2·sp1·sp0 / (sp0² + sp1²)
    let lp_relative = 2.0 * sp1 * sp0 / (sp0 * sp0 + sp1 * sp1);

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

    /// Regression test for BUG-qr9: price_lower/price_upper must be decimal-scaled
    /// to the same unit as entry_price/price_current.
    ///
    /// SOL/USDC example: price_current ≈ $84.225, range ≈ [$75, $95].
    /// With correct scaling (all in USD), IL is negative when price moved from entry.
    /// With raw sqrt_q64 values (~0.084–0.096), both prices collapse to range
    /// boundaries and IL returns ~0 (the bug).
    #[test]
    fn test_il_nonzero_with_scaled_range() {
        // Simulated SOL/USDC watch loop values after fix (all decimal-scaled USD)
        let entry_price = 85.122_f64;
        let price_current = 84.225_f64;
        let price_lower_scaled = 75.0_f64; // e.g. sqrt_q64_to_price(...) * 1000
        let price_upper_scaled = 95.0_f64;

        let il = compute_il(entry_price, price_current, price_lower_scaled, price_upper_scaled);
        assert!(
            il < 0.0,
            "IL must be negative when price moved from entry: got {}",
            il
        );

        // Demonstrate the bug: unscaled range (raw ~0.084–0.096) collapses IL to ~0
        let price_lower_raw = 0.084_f64; // raw sqrt_q64_to_price output (no * 1000)
        let price_upper_raw = 0.096_f64;
        let il_bugged = compute_il(entry_price, price_current, price_lower_raw, price_upper_raw);
        // Both entry and current are clamped to pb (upper boundary ≈ 0.31), yielding IL≈0
        assert!(
            il_bugged.abs() < 1e-6,
            "Unscaled range should collapse IL to ~0 (bug reproduction): got {}",
            il_bugged
        );
    }
}
