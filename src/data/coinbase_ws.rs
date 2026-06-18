use std::time::Instant;
use tokio::sync::broadcast;
use tracing::{info, warn};

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::data::cex_ws::{CexPrice, CexPriceState, validate_quote};

const COINBASE_WS_URL: &str = "wss://ws-feed.exchange.coinbase.com";

/// Exponential backoff bounds — mirror cex_ws constants.
const CONNECT_RETRY_BASE: std::time::Duration = std::time::Duration::from_secs(5);
const CONNECT_RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(300);

/// If `stream.next()` does not yield a frame within this window the feed is
/// considered stale and we reconnect. Coinbase can go silent without closing
/// the socket, so wrapping each `next()` in a timeout is the only defence.
const FEED_STALE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Serde target for a Coinbase ticker message.
#[derive(serde::Deserialize)]
struct CoinbaseTicker {
    #[serde(rename = "type")]
    msg_type: Option<String>,
    product_id: Option<String>,
    best_bid: Option<String>,
    best_ask: Option<String>,
}

/// Parse a Coinbase WebSocket message.
///
/// Returns `Some(mid)` only for a `ticker` message for `expected_product` that
/// carries sane (non-crossed, finite, positive) best_bid/best_ask fields.
/// Returns `None` for subscription confirmations, heartbeats, error frames,
/// wrong products, missing or non-numeric fields, or quotes that fail
/// [`validate_quote`].
fn parse_ticker_mid(raw: &str, expected_product: &str) -> Option<f64> {
    let msg: CoinbaseTicker = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return None,
    };

    // Must be a "ticker" message.
    if msg.msg_type.as_deref() != Some("ticker") {
        return None;
    }

    // Product-ID guard — do not trust server routing.
    if msg.product_id.as_deref() != Some(expected_product) {
        return None;
    }

    let bid_str = msg.best_bid.as_deref()?;
    let ask_str = msg.best_ask.as_deref()?;

    let bid: f64 = bid_str.parse().ok()?;
    let ask: f64 = ask_str.parse().ok()?;

    if !validate_quote(bid, ask) {
        warn!(
            "coinbase_ws: invalid quote bid='{}' ask='{}', skipping",
            bid_str, ask_str
        );
        return None;
    }

    Some((bid + ask) / 2.0)
}

/// Watch the Coinbase public WebSocket feed for `product_id` (e.g. `"SOL-USD"`).
///
/// Writes mid-price = (best_bid + best_ask) / 2 into `state` on every accepted
/// ticker update, and publishes the mid to the metrics layer via
/// [`crate::metrics::record_price`].
///
/// Reconnect behaviour mirrors [`crate::data::cex_ws::watch_binance_price`]:
/// - Exponential backoff on connect failures (`CONNECT_RETRY_BASE` ..
///   `CONNECT_RETRY_MAX`), reset on a healthy connect.
/// - Each `stream.next()` is wrapped in a `FEED_STALE_TIMEOUT` timeout; silence
///   triggers a warn + reconnect without touching the backoff counter.
/// - `shutdown` is checked before every connect attempt and honoured during
///   backoff sleeps via `tokio::select!`.
///
/// Returns cleanly when `shutdown` fires.
pub async fn watch_coinbase_price(
    product_id: String,
    state: CexPriceState,
    mut shutdown: broadcast::Receiver<()>,
) {
    let mut backoff = CONNECT_RETRY_BASE;

    loop {
        // Honour shutdown before every connect attempt.
        if shutdown.try_recv().is_ok() {
            info!("coinbase_ws: shutdown received before connect, exiting");
            return;
        }

        match run_session(&product_id, &state, &mut shutdown).await {
            SessionEnd::Shutdown => {
                info!("coinbase_ws: shutdown received, exiting");
                return;
            }
            SessionEnd::ConnectError(reason) => {
                warn!(
                    "coinbase_ws: connect/subscribe failed: {}; retrying in {}s",
                    reason,
                    backoff.as_secs()
                );
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown.recv() => {
                        info!("coinbase_ws: shutdown received during backoff, exiting");
                        return;
                    }
                }
                backoff = (backoff * 2).min(CONNECT_RETRY_MAX);
            }
            SessionEnd::Ended { reason, healthy } => {
                if healthy {
                    // The session delivered at least one valid ticker before it
                    // ended — treat it as a genuine mid-life drop: reset backoff
                    // and reconnect immediately.
                    warn!("coinbase_ws: live session ended ({}), reconnecting", reason);
                    backoff = CONNECT_RETRY_BASE;
                } else {
                    // Connected but never produced a price before failing (e.g.
                    // server accepts the upgrade then immediately closes / errors,
                    // or rejects the subscribe by dropping the stream). Without a
                    // backoff + sleep this would hammer Coinbase in a tight loop
                    // and risk getting the feed rate-limited dark. Grow the
                    // backoff exactly like a connect failure.
                    warn!(
                        "coinbase_ws: session failed before first price ({}); retrying in {}s",
                        reason,
                        backoff.as_secs()
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {}
                        _ = shutdown.recv() => {
                            info!("coinbase_ws: shutdown received during backoff, exiting");
                            return;
                        }
                    }
                    backoff = (backoff * 2).min(CONNECT_RETRY_MAX);
                }
            }
        }
    }
}

/// How a single session ended.
enum SessionEnd {
    Shutdown,
    /// `connect_async` or the subscribe frame failed — keep/grow backoff.
    ConnectError(String),
    /// A connected session ended (clean close, stream error, or staleness).
    ///
    /// `healthy` distinguishes a genuine mid-life drop (at least one valid
    /// ticker was received — reset backoff, reconnect immediately) from a
    /// session that connected but failed before producing any price (keep and
    /// grow the backoff, so a server that accepts the upgrade then immediately
    /// errors is not hammered in a tight loop).
    Ended {
        reason: String,
        healthy: bool,
    },
}

/// Drive one WebSocket session: connect, subscribe, read until done.
async fn run_session(
    product_id: &str,
    state: &CexPriceState,
    shutdown: &mut broadcast::Receiver<()>,
) -> SessionEnd {
    let (ws_stream, _) = match connect_async(COINBASE_WS_URL).await {
        Ok(pair) => pair,
        Err(e) => return SessionEnd::ConnectError(format!("connect_async: {e}")),
    };

    let (mut write, mut read) = ws_stream.split();

    // Send the subscribe frame.
    let subscribe = serde_json::json!({
        "type": "subscribe",
        "product_ids": [product_id],
        "channels": ["ticker"]
    });
    if let Err(e) = write.send(Message::Text(subscribe.to_string())).await {
        return SessionEnd::ConnectError(format!("subscribe send: {e}"));
    }

    info!(
        "coinbase_ws: connected, subscribed to ticker for {}",
        product_id
    );

    // Whether this session has produced at least one valid price. Drives the
    // backoff decision on exit: a drop after real data is a healthy mid-life
    // reconnect; a drop before any data keeps the backoff growing.
    let mut got_price = false;

    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                return SessionEnd::Shutdown;
            }

            result = tokio::time::timeout(FEED_STALE_TIMEOUT, read.next()) => {
                match result {
                    Err(_elapsed) => {
                        // timeout — feed went silent
                        return SessionEnd::Ended {
                            reason: format!("no frame for >{}s", FEED_STALE_TIMEOUT.as_secs()),
                            healthy: got_price,
                        };
                    }
                    Ok(None) => {
                        return SessionEnd::Ended {
                            reason: "stream closed".to_string(),
                            healthy: got_price,
                        };
                    }
                    Ok(Some(Err(e))) => {
                        warn!("coinbase_ws: stream error: {}", e);
                        return SessionEnd::Ended {
                            reason: format!("stream error: {e}"),
                            healthy: got_price,
                        };
                    }
                    Ok(Some(Ok(Message::Text(text)))) => {
                        if let Some(mid) = parse_ticker_mid(&text, product_id) {
                            {
                                let mut guard =
                                    state.write().unwrap_or_else(|p| p.into_inner());
                                *guard = Some(CexPrice {
                                    price: mid,
                                    updated_at: Instant::now(),
                                });
                            }
                            crate::metrics::record_price(crate::data::Source::Coinbase, mid);
                            got_price = true;
                        }
                    }
                    Ok(Some(Ok(Message::Close(_)))) => {
                        return SessionEnd::Ended {
                            reason: "server closed connection".to_string(),
                            healthy: got_price,
                        };
                    }
                    Ok(Some(Ok(Message::Ping(data)))) => {
                        // Reply to server-initiated pings so the connection stays alive.
                        if let Err(e) = write.send(Message::Pong(data)).await {
                            return SessionEnd::Ended {
                                reason: format!("pong send: {e}"),
                                healthy: got_price,
                            };
                        }
                    }
                    Ok(Some(Ok(_))) => {
                        // Binary, Pong, Frame — ignore.
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_ticker_mid ──────────────────────────────────────────────────────

    #[test]
    fn valid_ticker_returns_mid() {
        let raw = r#"{
            "type":"ticker",
            "product_id":"SOL-USD",
            "best_bid":"99.0",
            "best_ask":"101.0"
        }"#;
        let mid = parse_ticker_mid(raw, "SOL-USD").expect("should be Some");
        assert!((mid - 100.0).abs() < 1e-9, "expected 100.0, got {mid}");
    }

    #[test]
    fn valid_ticker_mid_is_arithmetic_mean() {
        let raw =
            r#"{"type":"ticker","product_id":"SOL-USD","best_bid":"120.4","best_ask":"120.6"}"#;
        let mid = parse_ticker_mid(raw, "SOL-USD").expect("Some");
        assert!((mid - 120.5).abs() < 1e-9);
    }

    #[test]
    fn non_ticker_type_subscriptions_returns_none() {
        let raw = r#"{"type":"subscriptions","channels":[]}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn non_ticker_type_heartbeat_returns_none() {
        let raw = r#"{"type":"heartbeat","product_id":"SOL-USD","sequence":1}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn non_ticker_type_error_returns_none() {
        let raw = r#"{"type":"error","message":"Bad request","reason":"invalid"}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn wrong_product_id_returns_none() {
        let raw =
            r#"{"type":"ticker","product_id":"BTC-USD","best_bid":"30000.0","best_ask":"30001.0"}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn missing_best_bid_returns_none() {
        let raw = r#"{"type":"ticker","product_id":"SOL-USD","best_ask":"101.0"}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn missing_best_ask_returns_none() {
        let raw = r#"{"type":"ticker","product_id":"SOL-USD","best_bid":"99.0"}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn non_numeric_bid_returns_none() {
        let raw = r#"{"type":"ticker","product_id":"SOL-USD","best_bid":"not-a-number","best_ask":"101.0"}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn non_numeric_ask_returns_none() {
        let raw = r#"{"type":"ticker","product_id":"SOL-USD","best_bid":"99.0","best_ask":"nope"}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn nan_bid_returns_none() {
        let raw = r#"{"type":"ticker","product_id":"SOL-USD","best_bid":"NaN","best_ask":"101.0"}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn inf_ask_returns_none() {
        let raw = r#"{"type":"ticker","product_id":"SOL-USD","best_bid":"99.0","best_ask":"inf"}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn negative_bid_returns_none() {
        let raw =
            r#"{"type":"ticker","product_id":"SOL-USD","best_bid":"-10.0","best_ask":"101.0"}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn zero_ask_returns_none() {
        let raw = r#"{"type":"ticker","product_id":"SOL-USD","best_bid":"99.0","best_ask":"0"}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn crossed_book_bid_greater_than_ask_returns_none() {
        let raw =
            r#"{"type":"ticker","product_id":"SOL-USD","best_bid":"102.0","best_ask":"100.0"}"#;
        assert!(parse_ticker_mid(raw, "SOL-USD").is_none());
    }

    #[test]
    fn null_bid_or_ask_returns_none() {
        // Coinbase can send null bid/ask during a trading halt; Option<String>
        // deserializes JSON null to None and as_deref()? must propagate it.
        let null_bid =
            r#"{"type":"ticker","product_id":"SOL-USD","best_bid":null,"best_ask":"101.0"}"#;
        assert!(parse_ticker_mid(null_bid, "SOL-USD").is_none());
        let null_ask =
            r#"{"type":"ticker","product_id":"SOL-USD","best_bid":"99.0","best_ask":null}"#;
        assert!(parse_ticker_mid(null_ask, "SOL-USD").is_none());
    }

    #[test]
    fn malformed_json_returns_none_no_panic() {
        assert!(parse_ticker_mid("not json at all {{{", "SOL-USD").is_none());
        assert!(parse_ticker_mid("", "SOL-USD").is_none());
        assert!(parse_ticker_mid("{", "SOL-USD").is_none());
    }
}
