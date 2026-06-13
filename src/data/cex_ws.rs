use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{info, warn};

use binance_sdk::config::ConfigurationWebsocketStreams;
use binance_sdk::spot::SpotWsStreams;
use binance_sdk::spot::websocket_streams::{BookTickerParams, BookTickerResponse};

/// Exponential backoff bounds for initial-connect / subscription attempts.
/// binance-sdk v45 handles in-session reconnects, but a failure *before* the
/// connection is established (DNS, TLS, auth, malformed params …) is surfaced
/// as a one-shot error. A fixed retry hammers Binance's handshake rate-limit
/// during an outage, so we grow the delay exponentially up to a cap.
const CONNECT_RETRY_BASE: Duration = Duration::from_secs(5);
const CONNECT_RETRY_MAX: Duration = Duration::from_secs(300);

/// How often, and after how long without a fresh quote, we treat a live
/// session as dead and force a reconnect. binance-sdk v45 makes only a single
/// in-session reconnect attempt per drop, so a longer outage can leave the
/// session permanently silent with no error — without this watchdog the feed
/// would stay dead until process restart.
const LIVENESS_CHECK_INTERVAL: Duration = Duration::from_secs(15);
const FEED_STALE_TIMEOUT: Duration = Duration::from_secs(60);

/// How a live session ended.
enum SessionEnd {
    Shutdown,
    Stale,
}

pub struct CexPrice {
    pub price: f64,
    pub updated_at: Instant,
}

pub type CexPriceState = Arc<RwLock<Option<CexPrice>>>;

/// Apply a BookTicker update to shared state. Returns true if state was written,
/// false if the incoming data was missing or failed to parse (logs a warn! in that case).
///
/// binance-sdk v45 `BookTickerResponse` exposes bid/ask as `b: Option<String>` and
/// `a: Option<String>` (short field names — upstream mirrors the raw Binance protocol).
/// `None` or non-numeric inputs are treated as malformed; state is left unchanged.
fn apply_book_ticker(bid: Option<&str>, ask: Option<&str>, state: &CexPriceState) -> bool {
    let bid_str = match bid {
        Some(v) => v,
        None => {
            warn!("cex_ws: missing bid field, skipping");
            return false;
        }
    };
    let ask_str = match ask {
        Some(v) => v,
        None => {
            warn!("cex_ws: missing ask field, skipping");
            return false;
        }
    };
    let bid_f: f64 = match bid_str.parse() {
        Ok(v) => v,
        Err(_) => {
            warn!("cex_ws: non-numeric bid '{}', skipping", bid_str);
            return false;
        }
    };
    let ask_f: f64 = match ask_str.parse() {
        Ok(v) => v,
        Err(_) => {
            warn!("cex_ws: non-numeric ask '{}', skipping", ask_str);
            return false;
        }
    };
    // `f64::parse` happily accepts "NaN"/"inf"/negatives. A NaN mid-price
    // poisons P&L and silently disables threshold risk checks (NaN comparisons
    // are always false), so reject anything that is not a sane quote.
    if !(bid_f.is_finite() && ask_f.is_finite() && bid_f > 0.0 && ask_f > 0.0 && bid_f <= ask_f) {
        warn!(
            "cex_ws: invalid quote bid='{}' ask='{}', skipping",
            bid_str, ask_str
        );
        return false;
    }
    let mid = (bid_f + ask_f) / 2.0;
    let mut guard = state.write().unwrap_or_else(|p| p.into_inner());
    *guard = Some(CexPrice {
        price: mid,
        updated_at: Instant::now(),
    });
    true
}

/// Watch Binance bookTicker stream for `symbol` (case-insensitive; lowercased by SDK params).
/// Writes mid-price = (bid + ask) / 2 into `state` on every update.
///
/// Reconnect / ping-pong / backoff *within a live session* are handled by
/// binance-sdk v45. However, an **initial-connect failure** (DNS error,
/// TLS handshake failure, config / params build error, book_ticker subscribe
/// error) is surfaced as a one-shot `Result::Err` and is NOT retried by the
/// SDK. We therefore wrap the connect / subscribe dance in our own
/// reconnect-loop with a [`CONNECT_RETRY_DELAY`] backoff so that transient
/// network issues at startup do not permanently disable the CEX price feed.
///
/// Returns on shutdown broadcast.
pub async fn watch_binance_price(
    symbol: String,
    state: CexPriceState,
    mut shutdown: broadcast::Receiver<()>,
) {
    let mut backoff = CONNECT_RETRY_BASE;

    loop {
        // Check for shutdown before every connect attempt so we don't
        // reconnect forever after the caller has torn everything down.
        if shutdown.try_recv().is_ok() {
            info!("cex_ws: shutdown received before connect, exiting");
            return;
        }

        match try_connect_and_subscribe(&symbol, &state).await {
            Ok(connection) => {
                // Healthy connect — reset the connect backoff.
                backoff = CONNECT_RETRY_BASE;

                // Hold the connection until shutdown OR the feed goes stale
                // (SDK reconnect exhausted / stream silently died).
                let end = monitor_session(&state, &mut shutdown).await;
                if let Err(e) = connection.disconnect().await {
                    warn!("cex_ws: disconnect error: {}", e);
                }
                match end {
                    SessionEnd::Shutdown => {
                        info!("cex_ws: shutdown received, disconnecting");
                        return;
                    }
                    SessionEnd::Stale => {
                        warn!(
                            "cex_ws: no fresh quote for >{}s -- reconnecting",
                            FEED_STALE_TIMEOUT.as_secs()
                        );
                        // Reconnect immediately (this is not a connect failure).
                        continue;
                    }
                }
            }
            Err(e) => {
                warn!(
                    "cex_ws: initial connect/subscribe failed: {}; retrying in {}s",
                    e,
                    backoff.as_secs()
                );
                // Wait for either the backoff to elapse or shutdown, whichever
                // comes first — avoids a pointless retry right before we exit.
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown.recv() => {
                        info!("cex_ws: shutdown received during backoff, exiting");
                        return;
                    }
                }
                backoff = (backoff * 2).min(CONNECT_RETRY_MAX);
            }
        }
    }
}

/// Hold a live session until shutdown fires or the feed goes stale.
///
/// Polls `state.updated_at` on a fixed interval: if no fresh quote has arrived
/// within [`FEED_STALE_TIMEOUT`] the session is considered dead and the caller
/// reconnects. A never-populated state (no message since connect) is judged
/// against the time since monitoring began.
async fn monitor_session(
    state: &CexPriceState,
    shutdown: &mut broadcast::Receiver<()>,
) -> SessionEnd {
    let session_started = Instant::now();
    let mut ticker = tokio::time::interval(LIVENESS_CHECK_INTERVAL);
    ticker.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            // Either Ok(()) or a closed/lagged channel means "stop".
            _ = shutdown.recv() => return SessionEnd::Shutdown,
            _ = ticker.tick() => {
                let stale = {
                    let guard = state.read().unwrap_or_else(|p| p.into_inner());
                    match guard.as_ref() {
                        Some(cp) => cp.updated_at.elapsed() > FEED_STALE_TIMEOUT,
                        None => session_started.elapsed() > FEED_STALE_TIMEOUT,
                    }
                };
                if stale {
                    return SessionEnd::Stale;
                }
            }
        }
    }
}

/// Build config / connect / subscribe to bookTicker.
///
/// Returns the live `WebsocketStreamsConnection` on success (caller is
/// responsible for `disconnect()`), or an error string describing which
/// step failed so the reconnect-loop can log it.
async fn try_connect_and_subscribe(
    symbol: &str,
    state: &CexPriceState,
) -> Result<binance_sdk::spot::websocket_streams::WebsocketStreams, String> {
    let cfg = ConfigurationWebsocketStreams::builder()
        .build()
        .map_err(|e| format!("config build error: {e}"))?;
    let client = SpotWsStreams::production(cfg);
    let connection = client
        .connect()
        .await
        .map_err(|e| format!("connect error: {e}"))?;
    let params = match BookTickerParams::builder(symbol.to_lowercase()).build() {
        Ok(p) => p,
        Err(e) => {
            let _ = connection.disconnect().await;
            return Err(format!("params build error: {e}"));
        }
    };
    let stream = match connection.book_ticker(params).await {
        Ok(s) => s,
        Err(e) => {
            let _ = connection.disconnect().await;
            return Err(format!("book_ticker subscribe error: {e}"));
        }
    };
    info!("cex_ws: connected, subscribed to bookTicker");

    let state_cb = state.clone();
    let expected_symbol = symbol.to_uppercase();
    stream.on_message(move |data: BookTickerResponse| {
        // Don't trust SDK routing alone: a price that moves funds must come
        // from the symbol we actually subscribed to.
        if data
            .s
            .as_deref()
            .map(|s| s.eq_ignore_ascii_case(&expected_symbol))
            != Some(true)
        {
            warn!(
                "cex_ws: bookTicker for unexpected symbol {:?} (want {}), skipping",
                data.s, expected_symbol
            );
            return;
        }
        apply_book_ticker(data.b.as_deref(), data.a.as_deref(), &state_cb);
    });

    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cex_price_struct_has_public_fields() {
        let p = CexPrice {
            price: 10.0,
            updated_at: Instant::now(),
        };
        assert_eq!(p.price, 10.0);
        let _ = p.updated_at;
    }

    #[test]
    fn apply_book_ticker_updates_state() {
        let state: CexPriceState = Arc::new(RwLock::new(None));
        let updated = apply_book_ticker(Some("100.0"), Some("102.0"), &state);
        assert!(updated);
        let guard = state.read().expect("read");
        let cp = guard.as_ref().expect("some");
        assert!((cp.price - 101.0).abs() < 1e-9);
    }

    #[test]
    fn apply_book_ticker_ignores_malformed_and_keeps_state() {
        let state: CexPriceState = Arc::new(RwLock::new(None));

        // non-numeric bid
        let updated = apply_book_ticker(Some("not-a-number"), Some("1.0"), &state);
        assert!(!updated);
        assert!(state.read().expect("read").is_none());

        // non-numeric ask
        let updated2 = apply_book_ticker(Some("1.0"), Some("nope"), &state);
        assert!(!updated2);
        assert!(state.read().expect("read").is_none());

        // missing bid
        let updated3 = apply_book_ticker(None, Some("1.0"), &state);
        assert!(!updated3);
        assert!(state.read().expect("read").is_none());

        // missing ask
        let updated4 = apply_book_ticker(Some("1.0"), None, &state);
        assert!(!updated4);
        assert!(state.read().expect("read").is_none());
    }

    #[test]
    fn apply_book_ticker_rejects_non_finite_and_non_positive_quotes() {
        let state: CexPriceState = Arc::new(RwLock::new(None));

        // f64::parse accepts all of these — the validation layer must not.
        for (bid, ask) in [
            ("NaN", "100.0"),
            ("100.0", "NaN"),
            ("inf", "100.0"),
            ("100.0", "inf"),
            ("-inf", "-inf"),
            ("-100.0", "100.0"),
            ("100.0", "-100.0"),
            ("0", "100.0"),
            ("100.0", "0"),
        ] {
            let updated = apply_book_ticker(Some(bid), Some(ask), &state);
            assert!(!updated, "quote bid={} ask={} must be rejected", bid, ask);
            assert!(state.read().expect("read").is_none());
        }
    }

    #[test]
    fn apply_book_ticker_rejects_crossed_book() {
        let state: CexPriceState = Arc::new(RwLock::new(None));
        let updated = apply_book_ticker(Some("102.0"), Some("100.0"), &state);
        assert!(!updated);
        assert!(state.read().expect("read").is_none());
    }
}
