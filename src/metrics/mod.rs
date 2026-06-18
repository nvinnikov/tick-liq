//! Observability transport layer — Prometheus / VictoriaMetrics metrics.
//!
//! This module is self-contained: it knows nothing about RPC, pools, or DB.
//! Call [`init_from_env`] once at startup (from inside a tokio runtime) to
//! install the global recorder; after that all emit helpers are safe to call
//! from any thread and are cheap no-ops when no recorder is installed.

use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use metrics_exporter_prometheus::PrometheusBuilder;

use crate::data::Source;

// ── Public types ─────────────────────────────────────────────────────────────

/// Snapshot of a watched position's state, emitted as gauges labelled by mint.
pub struct PositionMetrics {
    pub mint: String,
    pub value_usd: f64,
    pub pnl_usd: f64,
    pub fees_usd: f64,
    pub il_usd: f64,
    pub delta: f64,
    pub gamma: f64,
    pub in_range: bool,
    pub rebalance_signal: bool,
}

// ── Init ─────────────────────────────────────────────────────────────────────

/// Read environment variables and install the global Prometheus recorder.
///
/// - `METRICS_LISTEN` (e.g. `0.0.0.0:9100`) → pull/scrape mode.
/// - `METRICS_PUSH_URL` → push gateway mode (VictoriaMetrics `/api/v1/import/prometheus`).
///   Interval controlled by `METRICS_PUSH_INTERVAL_SECS` (default 15).
/// - Both set → listener wins, push URL is ignored (warning logged).
/// - Neither set → metrics disabled, returns `Ok(false)`.
///
/// Returns `Ok(true)` on successful install, `Ok(false)` when disabled.
///
/// # Errors
///
/// Returns an error if the address/URL fails to parse or if the recorder
/// installation fails (e.g. a global recorder is already set).
pub fn init_from_env() -> anyhow::Result<bool> {
    let listen = std::env::var("METRICS_LISTEN").ok();
    let push_url = std::env::var("METRICS_PUSH_URL").ok();

    match (listen, push_url) {
        (Some(addr_str), push_opt) => {
            if push_opt.is_some() {
                tracing::warn!(
                    "METRICS_LISTEN and METRICS_PUSH_URL are both set; \
                     using HTTP listener, push gateway will be ignored"
                );
            }
            let addr = SocketAddr::from_str(&addr_str).map_err(|e| {
                anyhow::anyhow!(
                    "METRICS_LISTEN '{}' is not a valid SocketAddr: {}",
                    addr_str,
                    e
                )
            })?;
            PrometheusBuilder::new()
                .with_http_listener(addr)
                .install()
                .map_err(|e| anyhow::anyhow!("failed to install Prometheus listener: {}", e))?;
            describe_metrics();
            tracing::info!(listen = %addr, "metrics: HTTP listener installed");
            Ok(true)
        }

        (None, Some(url)) => {
            // A malformed interval must not silently fall back — an operator
            // who typo'd "30s" deserves to know their setting was ignored.
            let interval_secs: u64 = match std::env::var("METRICS_PUSH_INTERVAL_SECS") {
                Ok(raw) => raw.parse().unwrap_or_else(|_| {
                    tracing::warn!(
                        raw = %raw,
                        "METRICS_PUSH_INTERVAL_SECS is not a valid u64; falling back to 15"
                    );
                    15
                }),
                Err(_) => 15,
            };
            PrometheusBuilder::new()
                .with_push_gateway(url.clone(), Duration::from_secs(interval_secs), None, None)
                .map_err(|e| {
                    anyhow::anyhow!("METRICS_PUSH_URL '{}' is not a valid URI: {}", url, e)
                })?
                .install()
                .map_err(|e| anyhow::anyhow!("failed to install Prometheus push gateway: {}", e))?;
            describe_metrics();
            tracing::info!(
                push_url = %url,
                interval_secs,
                "metrics: push gateway installed"
            );
            Ok(true)
        }

        (None, None) => {
            tracing::info!("metrics disabled (set METRICS_LISTEN or METRICS_PUSH_URL to enable)");
            Ok(false)
        }
    }
}

// ── Describe ─────────────────────────────────────────────────────────────────

fn describe_metrics() {
    metrics::describe_gauge!("tickliq_price_mid", "Mid-price for a given source (USD)");
    metrics::describe_gauge!(
        "tickliq_price_deviation_bps",
        "Price deviation from Binance reference price (basis points)"
    );
    metrics::describe_gauge!("tickliq_feed_up", "1 if the price feed is live, 0 if down");
    metrics::describe_gauge!(
        "tickliq_feed_staleness_seconds",
        "Seconds since the last successful price update from this feed"
    );
    metrics::describe_gauge!(
        "tickliq_position_value_usd",
        "Total current value of the LP position in USD"
    );
    metrics::describe_gauge!(
        "tickliq_pnl_usd",
        "Net P&L of the position (fees minus IL) in USD"
    );
    metrics::describe_gauge!(
        "tickliq_fees_earned_usd",
        "Cumulative fees earned by the position in USD"
    );
    metrics::describe_gauge!(
        "tickliq_il_usd",
        "Impermanent loss of the position in USD (negative)"
    );
    metrics::describe_gauge!(
        "tickliq_delta",
        "LP position delta (sensitivity to price change)"
    );
    metrics::describe_gauge!(
        "tickliq_gamma",
        "LP position gamma (second-order price sensitivity)"
    );
    metrics::describe_gauge!(
        "tickliq_in_range",
        "1 if the position is in range, 0 otherwise"
    );
    metrics::describe_gauge!(
        "tickliq_rebalance_signal",
        "1 if a rebalance signal has been triggered, 0 otherwise"
    );
}

// ── Emit helpers ─────────────────────────────────────────────────────────────

/// Record the mid-price from a given source.
pub fn record_price(source: Source, mid: f64) {
    metrics::gauge!("tickliq_price_mid", "source" => source.label()).set(mid);
}

/// Record the price deviation from the Binance reference price in basis points.
pub fn record_deviation(source: Source, bps: f64) {
    metrics::gauge!("tickliq_price_deviation_bps", "source" => source.label(), "ref" => "binance")
        .set(bps);
}

/// Record feed liveness and staleness for a given source.
pub fn record_feed(source: Source, up: bool, staleness_secs: f64) {
    metrics::gauge!("tickliq_feed_up", "source" => source.label()).set(if up { 1.0 } else { 0.0 });
    metrics::gauge!("tickliq_feed_staleness_seconds", "source" => source.label())
        .set(staleness_secs);
}

/// Record all gauges for a watched LP position.
pub fn record_position(snap: &PositionMetrics) {
    let mint: String = snap.mint.clone();
    metrics::gauge!("tickliq_position_value_usd", "mint" => mint.clone()).set(snap.value_usd);
    metrics::gauge!("tickliq_pnl_usd", "mint" => mint.clone()).set(snap.pnl_usd);
    metrics::gauge!("tickliq_fees_earned_usd", "mint" => mint.clone()).set(snap.fees_usd);
    metrics::gauge!("tickliq_il_usd", "mint" => mint.clone()).set(snap.il_usd);
    metrics::gauge!("tickliq_delta", "mint" => mint.clone()).set(snap.delta);
    metrics::gauge!("tickliq_gamma", "mint" => mint.clone()).set(snap.gamma);
    metrics::gauge!("tickliq_in_range", "mint" => mint.clone()).set(if snap.in_range {
        1.0
    } else {
        0.0
    });
    metrics::gauge!("tickliq_rebalance_signal", "mint" => mint).set(if snap.rebalance_signal {
        1.0
    } else {
        0.0
    });
}

// ── Pure helpers ─────────────────────────────────────────────────────────────

/// Compute the deviation of `price` from `reference` in basis points.
///
/// Returns `None` if either input is non-finite or if `reference <= 0.0`.
pub fn deviation_bps(price: f64, reference: f64) -> Option<f64> {
    if !price.is_finite() || !reference.is_finite() || reference <= 0.0 {
        return None;
    }
    Some((price - reference) / reference * 10_000.0)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── deviation_bps ────────────────────────────────────────────────────────

    #[test]
    fn deviation_bps_positive() {
        // price=101, ref=100 → (1/100)*10_000 = 100 bps
        let result = deviation_bps(101.0, 100.0).expect("should be Some");
        assert!(
            (result - 100.0).abs() < 1e-9,
            "expected 100 bps, got {result}"
        );
    }

    #[test]
    fn deviation_bps_negative() {
        // price=99, ref=100 → (-1/100)*10_000 = -100 bps
        let result = deviation_bps(99.0, 100.0).expect("should be Some");
        assert!(
            (result - (-100.0)).abs() < 1e-9,
            "expected -100 bps, got {result}"
        );
    }

    #[test]
    fn deviation_bps_equal() {
        // price == ref → 0 bps
        let result = deviation_bps(100.0, 100.0).expect("should be Some");
        assert!((result - 0.0).abs() < 1e-9, "expected 0 bps, got {result}");
    }

    #[test]
    fn deviation_bps_zero_reference() {
        assert!(deviation_bps(100.0, 0.0).is_none());
    }

    #[test]
    fn deviation_bps_negative_reference() {
        assert!(deviation_bps(100.0, -1.0).is_none());
    }

    #[test]
    fn deviation_bps_nan_price() {
        assert!(deviation_bps(f64::NAN, 100.0).is_none());
    }

    #[test]
    fn deviation_bps_inf_price() {
        assert!(deviation_bps(f64::INFINITY, 100.0).is_none());
    }

    #[test]
    fn deviation_bps_nan_reference() {
        assert!(deviation_bps(100.0, f64::NAN).is_none());
    }

    // ── Emission smoke test (local recorder — no global install) ─────────────

    #[test]
    fn emit_helpers_render_expected_metric_names_and_labels() {
        use metrics_exporter_prometheus::PrometheusBuilder;

        // Build a local recorder (no global install).
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        metrics::with_local_recorder(&recorder, || {
            // Price feed
            record_price(Source::Orca, 84.5);
            record_deviation(Source::Binance, 12.3);
            record_feed(Source::Coinbase, true, 0.5);

            // Position
            record_position(&PositionMetrics {
                mint: "ABC".into(),
                value_usd: 1000.0,
                pnl_usd: 50.0,
                fees_usd: 75.0,
                il_usd: -25.0,
                delta: -0.5,
                gamma: 0.01,
                in_range: true,
                rebalance_signal: false,
            });
        });

        let rendered = handle.render();

        // Price mid
        assert!(
            rendered.contains("tickliq_price_mid"),
            "missing tickliq_price_mid in:\n{rendered}"
        );
        assert!(
            rendered.contains("source=\"orca\""),
            "missing source=orca label in:\n{rendered}"
        );
        // Assert the concrete value made it through `.set()` — a no-op or
        // broken setter would still emit the metric name/label but drop this.
        assert!(
            rendered.contains("84.5"),
            "missing recorded price value 84.5 in:\n{rendered}"
        );

        // Deviation
        assert!(
            rendered.contains("tickliq_price_deviation_bps"),
            "missing tickliq_price_deviation_bps in:\n{rendered}"
        );
        assert!(
            rendered.contains("source=\"binance\""),
            "missing source=binance label in:\n{rendered}"
        );
        assert!(
            rendered.contains("ref=\"binance\""),
            "missing ref=binance label in:\n{rendered}"
        );

        // Feed
        assert!(
            rendered.contains("tickliq_feed_up"),
            "missing tickliq_feed_up in:\n{rendered}"
        );
        assert!(
            rendered.contains("tickliq_feed_staleness_seconds"),
            "missing tickliq_feed_staleness_seconds in:\n{rendered}"
        );
        assert!(
            rendered.contains("source=\"coinbase\""),
            "missing source=coinbase label in:\n{rendered}"
        );

        // Position
        assert!(
            rendered.contains("tickliq_pnl_usd"),
            "missing tickliq_pnl_usd in:\n{rendered}"
        );
        assert!(
            rendered.contains("tickliq_position_value_usd"),
            "missing tickliq_position_value_usd in:\n{rendered}"
        );
        assert!(
            rendered.contains("tickliq_fees_earned_usd"),
            "missing tickliq_fees_earned_usd in:\n{rendered}"
        );
        assert!(
            rendered.contains("tickliq_il_usd"),
            "missing tickliq_il_usd in:\n{rendered}"
        );
        assert!(
            rendered.contains("mint=\"ABC\""),
            "missing mint=ABC label in:\n{rendered}"
        );
    }
}
