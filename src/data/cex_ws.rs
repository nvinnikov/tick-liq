use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{info, warn};

use binance_sdk::config::ConfigurationWebsocketStreams;
use binance_sdk::spot::SpotWsStreams;
use binance_sdk::spot::websocket_streams::{BookTickerParams, BookTickerResponse};

/// Backoff delay between initial-connect / subscription attempts.
/// binance-sdk v45 handles in-session reconnects, but a failure *before*
/// the connection is established (DNS, TLS, auth, malformed params …)
/// is surfaced as a one-shot error — we retry it ourselves with a fixed
/// 5 s delay until shutdown fires.
const CONNECT_RETRY_DELAY: Duration = Duration::from_secs(5);

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
fn apply_book_ticker(
    bid: Option<&str>,
    ask: Option<&str>,
    state: &CexPriceState,
) -> bool {
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
    loop {
        // Check for shutdown before every connect attempt so we don't
        // reconnect forever after the caller has torn everything down.
        if shutdown.try_recv().is_ok() {
            info!("cex_ws: shutdown received before connect, exiting");
            return;
        }

        match try_connect_and_subscribe(&symbol, &state).await {
            Ok(connection) => {
                // Connection live; hold it until shutdown.
                let _ = shutdown.recv().await;
                info!("cex_ws: shutdown received, disconnecting");
                if let Err(e) = connection.disconnect().await {
                    warn!("cex_ws: disconnect error: {}", e);
                }
                return;
            }
            Err(e) => {
                warn!(
                    "cex_ws: initial connect/subscribe failed: {}; retrying in {}s",
                    e,
                    CONNECT_RETRY_DELAY.as_secs()
                );
                // Wait for either the backoff to elapse or shutdown, whichever
                // comes first — avoids a pointless retry right before we exit.
                tokio::select! {
                    _ = tokio::time::sleep(CONNECT_RETRY_DELAY) => {}
                    _ = shutdown.recv() => {
                        info!("cex_ws: shutdown received during backoff, exiting");
                        return;
                    }
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
    stream.on_message(move |data: BookTickerResponse| {
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
}
