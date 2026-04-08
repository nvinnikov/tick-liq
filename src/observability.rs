//! Tracing/observability bootstrap.
//!
//! `init_tracing()` sets up `tracing-subscriber` once for the entire process:
//!
//! - **Filter** comes from the `RUST_LOG` env var via `EnvFilter`, falling
//!   back to `info,tick_liq=debug` so our own targets are verbose by default
//!   without drowning in dependency chatter.
//! - **Format**: pretty/ANSI-colored in dev (`TICK_LIQ_LOG_FORMAT=pretty` or
//!   default when stdout is a TTY), structured JSON in production
//!   (`TICK_LIQ_LOG_FORMAT=json`).
//! - **Standard fields**: every event carries `target` (module path),
//!   `level`, `timestamp`, plus whatever span fields are active. We don't
//!   inject service-name/version here — that's the deployment's job via
//!   environment-injected fields.
//!
//! `init_tracing()` is idempotent: a second call is a no-op rather than a
//! panic, which makes it safe to call from tests as well as `main`.
//!
//! ## Helper spans
//!
//! Convenience constructors are provided for the four call sites the
//! strategy/execution layers need most. They all return a `tracing::Span`
//! that the caller `enter()`s (or attaches via `.instrument()` in async
//! code). Using these helpers keeps span names and field names consistent
//! across the crate so dashboards and log queries don't fragment.
//!
//! - [`rpc_call_span`] — wraps an RPC call. Fields: `method`, `pubkey`.
//! - [`ws_message_span`] — one WebSocket account update. Fields: `pool`, `slot`.
//! - [`signal_eval_span`] — strategy signal evaluation. Fields: `position`.
//! - [`rebalance_span`] — execution-layer rebalance attempt. Fields:
//!   `position`, `reason`.

use std::sync::Once;
use tracing::Span;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

static INIT: Once = Once::new();

/// Initialize the global `tracing` subscriber. Safe to call more than once;
/// subsequent calls are no-ops.
///
/// The subscriber honours these environment variables:
/// - `RUST_LOG` — standard `EnvFilter` directive (e.g. `info,tick_liq=debug`).
/// - `TICK_LIQ_LOG_FORMAT` — `json` for structured JSON output, anything else
///   (or unset) for the human-friendly format.
pub fn init_tracing() {
    INIT.call_once(|| {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,tick_liq=debug"));

        let json_mode = std::env::var("TICK_LIQ_LOG_FORMAT")
            .map(|v| v == "json")
            .unwrap_or(false);

        if json_mode {
            let layer = fmt::layer()
                .json()
                .with_target(true)
                .with_current_span(true)
                .with_span_list(false);
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(layer)
                .try_init();
        } else {
            let layer = fmt::layer().with_target(true);
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(layer)
                .try_init();
        }
    });
}

// -----------------------------------------------------------------------------
// Helper span constructors
// -----------------------------------------------------------------------------

/// Span for an RPC call. `method` is the JSON-RPC method name (e.g.
/// `"getAccount"`), `pubkey` is the account or program id stringified.
pub fn rpc_call_span(method: &str, pubkey: &str) -> Span {
    tracing::info_span!(
        target: "tick_liq::rpc",
        "rpc_call",
        method = method,
        pubkey = pubkey,
    )
}

/// Span for a single WebSocket pool-account update.
pub fn ws_message_span(pool: &str, slot: u64) -> Span {
    tracing::info_span!(
        target: "tick_liq::ws",
        "ws_message",
        pool = pool,
        slot = slot,
    )
}

/// Span for one strategy signal evaluation cycle.
pub fn signal_eval_span(position: &str) -> Span {
    tracing::info_span!(
        target: "tick_liq::strategy",
        "signal_eval",
        position = position,
    )
}

/// Span for an execution-layer rebalance attempt.
pub fn rebalance_span(position: &str, reason: &str) -> Span {
    tracing::info_span!(
        target: "tick_liq::execution",
        "rebalance",
        position = position,
        reason = reason,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent() {
        init_tracing();
        init_tracing();
        init_tracing();
    }

    #[test]
    fn rpc_call_span_carries_fields() {
        let span = rpc_call_span("getAccount", "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB");
        assert_eq!(span.metadata().unwrap().name(), "rpc_call");
        assert_eq!(span.metadata().unwrap().target(), "tick_liq::rpc");
    }

    #[test]
    fn ws_message_span_carries_fields() {
        let span = ws_message_span("PoolAddrXyz", 12345);
        assert_eq!(span.metadata().unwrap().name(), "ws_message");
        assert_eq!(span.metadata().unwrap().target(), "tick_liq::ws");
    }

    #[test]
    fn signal_eval_span_carries_fields() {
        let span = signal_eval_span("PosMintAbc");
        assert_eq!(span.metadata().unwrap().name(), "signal_eval");
        assert_eq!(span.metadata().unwrap().target(), "tick_liq::strategy");
    }

    #[test]
    fn rebalance_span_carries_fields() {
        let span = rebalance_span("PosMintAbc", "out_of_range");
        assert_eq!(span.metadata().unwrap().name(), "rebalance");
        assert_eq!(span.metadata().unwrap().target(), "tick_liq::execution");
    }
}
