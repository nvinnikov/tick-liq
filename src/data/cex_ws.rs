use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::sync::broadcast;
use tracing::{info, warn};

use binance_sdk::config::ConfigurationWebsocketStreams;
use binance_sdk::spot::SpotWsStreams;
use binance_sdk::spot::websocket_streams::{BookTickerParams, BookTickerResponse};

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
/// Reconnect / ping-pong / backoff handled by binance-sdk v45.
/// Returns on shutdown broadcast.
pub async fn watch_binance_price(
    symbol: String,
    state: CexPriceState,
    mut shutdown: broadcast::Receiver<()>,
) {
    let cfg = match ConfigurationWebsocketStreams::builder().build() {
        Ok(c) => c,
        Err(e) => {
            warn!("cex_ws: config build error: {}", e);
            return;
        }
    };
    let client = SpotWsStreams::production(cfg);
    let connection = match client.connect().await {
        Ok(c) => c,
        Err(e) => {
            warn!("cex_ws: connect error: {}", e);
            return;
        }
    };
    let params = match BookTickerParams::builder(symbol.to_lowercase()).build() {
        Ok(p) => p,
        Err(e) => {
            warn!("cex_ws: params build error: {}", e);
            let _ = connection.disconnect().await;
            return;
        }
    };
    let stream = match connection.book_ticker(params).await {
        Ok(s) => s,
        Err(e) => {
            warn!("cex_ws: book_ticker subscribe error: {}", e);
            let _ = connection.disconnect().await;
            return;
        }
    };
    info!("cex_ws: connected, subscribed to bookTicker");

    let state_cb = state.clone();
    stream.on_message(move |data: BookTickerResponse| {
        apply_book_ticker(data.b.as_deref(), data.a.as_deref(), &state_cb);
    });

    // Block until shutdown, then gracefully disconnect.
    let _ = shutdown.recv().await;
    info!("cex_ws: shutdown received, disconnecting");
    if let Err(e) = connection.disconnect().await {
        warn!("cex_ws: disconnect error: {}", e);
    }
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
