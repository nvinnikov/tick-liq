//! Integration tests for the slippage guard (strategy::slippage).
//!
//! These tests exercise the public API through the `tick_liq` crate,
//! covering: threshold pass/abort, custom threshold, edge cases, and monotonicity.

use tick_liq::strategy::slippage::{check_slippage, SlippageConfig, SlippageResult};

/// A small trade against a large realistic pool should pass the default 50 bps threshold.
#[test]
fn test_small_trade_large_pool_passes_default_threshold() {
    let config = SlippageConfig::default(); // 50 bps
    let result = check_slippage(
        25_000.0,            // $25k position value
        150.0,               // current price
        1_000_000_000_000,   // 1T liquidity — realistic for major Orca pools
        &config,
    );
    match result {
        SlippageResult::Ok { impact_bps } => {
            assert!(
                impact_bps < 50.0,
                "expected impact < 50 bps for small trade vs large pool, got {impact_bps}"
            );
        }
        SlippageResult::Abort { impact_bps, .. } => {
            panic!(
                "expected Ok for small trade vs large pool, got Abort with {impact_bps} bps"
            );
        }
    }
}

/// A large trade against a tiny pool must exceed the 50 bps threshold and abort.
#[test]
fn test_large_trade_tiny_pool_exceeds_threshold() {
    let config = SlippageConfig::default(); // 50 bps
    let result = check_slippage(
        100_000.0, // $100k position value
        150.0,     // current price
        1_000,     // tiny pool liquidity
        &config,
    );
    match result {
        SlippageResult::Abort { impact_bps, threshold_bps } => {
            assert!(
                impact_bps > 50.0,
                "expected impact > 50 bps for large trade vs tiny pool, got {impact_bps}"
            );
            assert_eq!(threshold_bps, 50, "threshold should match config default of 50");
        }
        SlippageResult::Ok { impact_bps } => {
            panic!(
                "expected Abort for large trade vs tiny pool, got Ok with {impact_bps} bps"
            );
        }
    }
}

/// Changing the threshold changes the outcome: same trade passes at 50 bps but aborts at 10 bps.
#[test]
fn test_custom_threshold_changes_outcome() {
    // Parameters tuned so impact lands around 20 bps:
    // medium trade ($5_000) against moderate pool (liquidity=1_000_000) at price=100.
    let position_value_usd = 5_000.0;
    let current_price = 100.0;
    let liquidity: u128 = 1_000_000;

    let loose_config = SlippageConfig { max_bps: 50 };
    let tight_config = SlippageConfig { max_bps: 10 };

    let loose_result = check_slippage(position_value_usd, current_price, liquidity, &loose_config);
    let tight_result = check_slippage(position_value_usd, current_price, liquidity, &tight_config);

    // The loose threshold (50 bps) should allow the trade.
    match &loose_result {
        SlippageResult::Ok { .. } => {}
        SlippageResult::Abort { impact_bps, .. } => {
            panic!("expected Ok with 50 bps threshold, got Abort with {impact_bps} bps — adjust test parameters");
        }
    }

    // The tight threshold (10 bps) should reject the same trade.
    match &tight_result {
        SlippageResult::Abort { .. } => {}
        SlippageResult::Ok { impact_bps } => {
            panic!("expected Abort with 10 bps threshold, got Ok with {impact_bps} bps — adjust test parameters");
        }
    }
}

/// Zero liquidity always results in Abort regardless of trade size.
#[test]
fn test_zero_liquidity_always_aborts() {
    let config = SlippageConfig::default();
    let result = check_slippage(
        1_000.0, // any non-zero trade
        100.0,
        0, // zero liquidity
        &config,
    );
    match result {
        SlippageResult::Abort { impact_bps, .. } => {
            assert!(
                impact_bps.is_infinite(),
                "expected infinite impact_bps for zero liquidity, got {impact_bps}"
            );
        }
        SlippageResult::Ok { .. } => {
            panic!("expected Abort for zero liquidity");
        }
    }
}

/// Zero trade size always results in Ok with approximately 0 bps impact.
#[test]
fn test_zero_trade_size_always_passes() {
    let config = SlippageConfig::default();
    let result = check_slippage(
        0.0,               // zero trade size
        100.0,
        1_000_000_000_000, // any non-zero liquidity
        &config,
    );
    match result {
        SlippageResult::Ok { impact_bps } => {
            assert!(
                impact_bps.abs() < 1e-9,
                "expected ~0 bps impact for zero trade size, got {impact_bps}"
            );
        }
        SlippageResult::Abort { .. } => {
            panic!("expected Ok for zero trade size");
        }
    }
}

/// Larger trades against the same pool produce strictly higher price impact.
#[test]
fn test_impact_increases_with_trade_size() {
    let config = SlippageConfig { max_bps: 10_000 }; // permissive so both trades get measured
    let price = 100.0;
    let liquidity: u128 = 1_000_000_000;

    let small_result = check_slippage(1_000.0, price, liquidity, &config);
    let large_result = check_slippage(50_000.0, price, liquidity, &config);

    let small_bps = match small_result {
        SlippageResult::Ok { impact_bps } => impact_bps,
        SlippageResult::Abort { impact_bps, .. } => impact_bps,
    };
    let large_bps = match large_result {
        SlippageResult::Ok { impact_bps } => impact_bps,
        SlippageResult::Abort { impact_bps, .. } => impact_bps,
    };

    assert!(
        large_bps > small_bps,
        "expected larger trade to have higher impact: small={small_bps} bps, large={large_bps} bps"
    );
}

/// The CLI default value must be exactly 50 bps.
#[test]
fn test_cli_default_is_50_bps() {
    assert_eq!(
        SlippageConfig::default().max_bps,
        50,
        "default max_bps should be 50 (matching the CLI --max-slippage-bps default)"
    );
}
