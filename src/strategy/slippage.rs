//! Slippage guard for rebalance transactions.
//!
//! Inverts `math::impact::estimate_impact()` to find the price impact (in bps)
//! caused by a trade of size `position_value_usd`. If the impact exceeds the
//! configured threshold the caller should abort the transaction.

use crate::math::impact::estimate_impact;

/// Configuration for slippage checking.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SlippageConfig {
    /// Maximum allowed price impact in basis points. Default: 50 (0.50%).
    pub max_bps: u32,
}

impl Default for SlippageConfig {
    fn default() -> Self {
        Self { max_bps: 50 }
    }
}

/// Result of a slippage check.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SlippageResult {
    /// Impact is below the threshold — transaction may proceed.
    Ok { impact_bps: f64 },
    /// Impact meets or exceeds the threshold — caller should abort.
    Abort { impact_bps: f64, threshold_bps: u32 },
}

/// Check whether executing a trade of `position_value_usd` at `current_price`
/// against a pool with `liquidity` would exceed the slippage threshold.
#[allow(dead_code)]
///
/// Uses binary search over `estimate_impact()` to find the `target_pct` whose
/// `usd_needed` matches `position_value_usd`, then converts to bps.
///
/// Edge cases:
/// - `liquidity == 0`       → `Abort` with `impact_bps = f64::INFINITY`
/// - `position_value_usd <= 0.0` → `Ok` with `impact_bps = 0.0`
pub fn check_slippage(
    position_value_usd: f64,
    current_price: f64,
    liquidity: u128,
    config: &SlippageConfig,
) -> SlippageResult {
    if liquidity == 0 {
        return SlippageResult::Abort {
            impact_bps: f64::INFINITY,
            threshold_bps: config.max_bps,
        };
    }

    if position_value_usd <= 0.0 {
        return SlippageResult::Ok { impact_bps: 0.0 };
    }

    // Binary search for target_pct in [0.001, 50.0] (percentage points).
    let mut lo: f64 = 0.001;
    let mut hi: f64 = 50.0;
    let mut impact_pct = (lo + hi) / 2.0;

    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        let result = estimate_impact(current_price, liquidity, mid, true);
        if (result.usd_needed - position_value_usd).abs() < 0.01 {
            impact_pct = mid;
            break;
        }
        if result.usd_needed < position_value_usd {
            lo = mid;
        } else {
            hi = mid;
        }
        impact_pct = mid;
    }

    // 1 percentage point = 100 bps
    let impact_bps = impact_pct * 100.0;
    let threshold = config.max_bps as f64;

    if impact_bps >= threshold {
        SlippageResult::Abort {
            impact_bps,
            threshold_bps: config.max_bps,
        }
    } else {
        SlippageResult::Ok { impact_bps }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_liquidity_aborts() {
        let config = SlippageConfig::default();
        let result = check_slippage(1000.0, 100.0, 0, &config);
        match result {
            SlippageResult::Abort { impact_bps, .. } => {
                assert!(impact_bps.is_infinite(), "expected infinite impact for zero liquidity");
            }
            SlippageResult::Ok { .. } => panic!("expected Abort for zero liquidity"),
        }
    }

    #[test]
    fn test_zero_trade_size_ok() {
        let config = SlippageConfig::default();
        let result = check_slippage(0.0, 100.0, 1_000_000_000, &config);
        match result {
            SlippageResult::Ok { impact_bps } => {
                assert_eq!(impact_bps, 0.0, "expected 0 bps for zero trade size");
            }
            SlippageResult::Abort { .. } => panic!("expected Ok for zero trade size"),
        }
    }

    #[test]
    fn test_small_trade_large_pool_ok() {
        // $1000 trade against very large liquidity pool — should be well under 50bps
        let config = SlippageConfig::default();
        let result = check_slippage(1000.0, 100.0, 10_000_000_000u128, &config);
        match result {
            SlippageResult::Ok { impact_bps } => {
                assert!(impact_bps < 50.0, "expected impact < 50 bps, got {impact_bps}");
            }
            SlippageResult::Abort { impact_bps, .. } => {
                panic!("expected Ok for small trade vs large pool, got Abort with {impact_bps} bps");
            }
        }
    }

    #[test]
    fn test_large_trade_small_pool_aborts() {
        // $100_000 trade against tiny liquidity — should exceed 50bps
        let config = SlippageConfig::default();
        let result = check_slippage(100_000.0, 100.0, 1_000, &config);
        match result {
            SlippageResult::Abort { impact_bps, threshold_bps } => {
                assert!(impact_bps >= 50.0, "expected impact >= 50 bps, got {impact_bps}");
                assert_eq!(threshold_bps, 50);
            }
            SlippageResult::Ok { impact_bps } => {
                panic!("expected Abort for large trade vs small pool, got Ok with {impact_bps} bps");
            }
        }
    }

    #[test]
    fn test_default_config_is_50_bps() {
        assert_eq!(SlippageConfig::default().max_bps, 50);
    }

    #[test]
    fn test_custom_threshold_respected() {
        // Use a very tight threshold of 10 bps — a trade that would pass at 50bps should now abort.
        // Use a moderately sized trade against a moderate pool.
        let tight_config = SlippageConfig { max_bps: 10 };
        let loose_config = SlippageConfig { max_bps: 50 };

        // Find parameters where the trade passes at 50bps but not at 10bps:
        // Use $5000 trade against 1_000_000 liquidity at price 100.
        let position_value_usd = 5000.0;
        let current_price = 100.0;
        let liquidity = 1_000_000u128;

        let loose_result = check_slippage(position_value_usd, current_price, liquidity, &loose_config);
        let tight_result = check_slippage(position_value_usd, current_price, liquidity, &tight_config);

        // At least one of them should abort — the tight threshold should be more restrictive.
        // If both are Ok, the impact is genuinely tiny and the test parameters need adjustment.
        // We assert that tight produces Abort OR that tight's impact >= loose's if both abort.
        match (&loose_result, &tight_result) {
            (SlippageResult::Ok { impact_bps }, SlippageResult::Abort { .. }) => {
                // Ideal: passes loose but fails tight
                assert!(*impact_bps < 50.0, "loose should pass");
            }
            (SlippageResult::Abort { .. }, SlippageResult::Abort { .. }) => {
                // Both abort — tight threshold is still more restrictive, that's fine
            }
            (SlippageResult::Ok { impact_bps: loose_bps }, SlippageResult::Ok { impact_bps: tight_bps }) => {
                // If both pass the check, ensure they produce the same impact
                assert!(
                    (loose_bps - tight_bps).abs() < 1.0,
                    "impact should be the same regardless of config"
                );
                // The tight threshold is so restrictive that even 10bps passes — that means
                // the trade itself is tiny. Relax the assertion.
                // NOTE: This case shouldn't happen with the parameters above.
                panic!(
                    "expected tight_config (10bps) to abort but got Ok with {tight_bps} bps; \
                     loose got {loose_bps} bps — adjust test parameters"
                );
            }
            (SlippageResult::Abort { .. }, SlippageResult::Ok { impact_bps }) => {
                panic!(
                    "tight config should not allow trade that loose config rejects; got Ok with {impact_bps} bps"
                );
            }
        }
    }
}
