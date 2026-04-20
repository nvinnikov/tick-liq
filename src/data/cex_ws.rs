use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};

pub(crate) const RECONNECT_BASE: Duration = Duration::from_secs(1);
pub(crate) const RECONNECT_MAX: Duration = Duration::from_secs(30);

pub struct CexPrice {
    pub price: f64,
    pub updated_at: Instant,
}

pub type CexPriceState = Arc<RwLock<Option<CexPrice>>>;

fn parse_book_ticker(text: &str) -> Option<f64> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let bid: f64 = v["b"].as_str()?.parse().ok()?;
    let ask: f64 = v["a"].as_str()?.parse().ok()?;
    Some((bid + ask) / 2.0)
}

fn build_stream_url(symbol: &str) -> String {
    format!(
        "wss://stream.binance.com:9443/ws/{}@bookTicker",
        symbol.to_lowercase()
    )
}

fn handle_frame(text: &str, state: &CexPriceState) {
    match parse_book_ticker(text) {
        Some(mid) => {
            let mut guard = state.write().unwrap_or_else(|p| p.into_inner());
            *guard = Some(CexPrice {
                price: mid,
                updated_at: Instant::now(),
            });
        }
        None => warn!("cex_ws: failed to parse bookTicker frame, skipping"),
    }
}

enum SessionResult {
    Shutdown,
    Reconnect { connected: bool },
}

/// Watch Binance bookTicker stream for `symbol` (case-insensitive; lowercased internally).
/// Writes mid-price = (bid + ask) / 2 into `state` on every update.
/// Auto-reconnects with exponential backoff. Returns on shutdown broadcast.
pub async fn watch_binance_price(
    symbol: String,
    state: CexPriceState,
    mut shutdown: broadcast::Receiver<()>,
) {
    let url = build_stream_url(&symbol);
    let mut backoff = RECONNECT_BASE;

    loop {
        if shutdown.try_recv().is_ok() {
            info!("cex_ws: shutdown received, exiting");
            return;
        }

        info!("cex_ws: connecting to {}", url);
        match run_binance_session(&url, &state, &mut shutdown).await {
            SessionResult::Shutdown => {
                info!("cex_ws: clean shutdown");
                return;
            }
            SessionResult::Reconnect { connected } => {
                if connected {
                    backoff = RECONNECT_BASE;
                }
                warn!("cex_ws: session ended, reconnecting in {:?}", backoff);
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown.recv() => {
                        info!("cex_ws: shutdown during backoff");
                        return;
                    }
                }
                backoff = (backoff * 2).min(RECONNECT_MAX);
            }
        }
    }
}

async fn run_binance_session(
    url: &str,
    state: &CexPriceState,
    shutdown: &mut broadcast::Receiver<()>,
) -> SessionResult {
    let (ws_stream, _) = match connect_async(url).await {
        Ok(pair) => pair,
        Err(e) => {
            warn!("cex_ws: connect error: {}", e);
            return SessionResult::Reconnect { connected: false };
        }
    };

    let (mut write, mut read) = ws_stream.split();
    info!("cex_ws: connected, waiting for bookTicker frames");

    loop {
        tokio::select! {
            biased;
            _ = shutdown.recv() => return SessionResult::Shutdown,
            msg = read.next() => {
                match msg {
                    None => return SessionResult::Reconnect { connected: true },
                    Some(Err(e)) => {
                        warn!("cex_ws: message error: {}", e);
                        return SessionResult::Reconnect { connected: true };
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if let Err(e) = write.send(Message::Pong(data)).await {
                            warn!("cex_ws: pong send error: {}", e);
                            return SessionResult::Reconnect { connected: true };
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {} // Binance pongs — ignore
                    Some(Ok(Message::Close(_))) => {
                        info!("cex_ws: server closed connection (expected after 24h)");
                        return SessionResult::Reconnect { connected: true };
                    }
                    Some(Ok(Message::Text(text))) => {
                        handle_frame(&text, state);
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        match std::str::from_utf8(&bytes) {
                            Ok(text) => handle_frame(text, state),
                            Err(_) => warn!("cex_ws: ignoring non-UTF-8 binary frame"),
                        }
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    #[test]
    fn parse_book_ticker_valid() {
        let msg = r#"{"u":1,"s":"SOLUSDT","b":"140.20","B":"1.0","a":"140.30","A":"2.0"}"#;
        let mid = super::parse_book_ticker(msg).expect("must parse");
        assert!((mid - 140.25).abs() < 1e-9);
    }

    #[test]
    fn parse_book_ticker_malformed_json() {
        assert!(super::parse_book_ticker("not json {{").is_none());
    }

    #[test]
    fn parse_book_ticker_missing_bid_ask() {
        assert!(super::parse_book_ticker(r#"{"u":1,"s":"X"}"#).is_none());
    }

    #[test]
    fn parse_book_ticker_non_numeric() {
        assert!(super::parse_book_ticker(r#"{"b":"abc","a":"1.0"}"#).is_none());
    }

    #[test]
    fn cex_price_struct_has_public_fields() {
        let p = super::CexPrice {
            price: 10.0,
            updated_at: std::time::Instant::now(),
        };
        assert_eq!(p.price, 10.0);
        let _ = p.updated_at;
    }

    #[test]
    fn build_stream_url_lowercases_symbol() {
        assert_eq!(
            super::build_stream_url("SOLUSDT"),
            "wss://stream.binance.com:9443/ws/solusdt@bookTicker"
        );
        assert_eq!(
            super::build_stream_url("solusdt"),
            "wss://stream.binance.com:9443/ws/solusdt@bookTicker"
        );
    }

    #[test]
    fn backoff_grows_then_resets() {
        let mut backoff = super::RECONNECT_BASE;

        // Two consecutive connect failures — backoff doubles each time.
        for _ in 0..2 {
            backoff = (backoff * 2).min(super::RECONNECT_MAX);
        }
        assert_eq!(backoff, std::time::Duration::from_secs(4));

        // Session connected then dropped → reset to base, then double.
        backoff = super::RECONNECT_BASE;
        backoff = (backoff * 2).min(super::RECONNECT_MAX);
        assert_eq!(backoff, std::time::Duration::from_secs(2));
    }

    #[test]
    fn backoff_saturates_at_max() {
        let mut backoff = super::RECONNECT_MAX;
        backoff = (backoff * 2).min(super::RECONNECT_MAX);
        assert_eq!(backoff, super::RECONNECT_MAX);
    }

    #[test]
    fn handle_frame_updates_state() {
        let state: super::CexPriceState = Arc::new(RwLock::new(None));
        let msg = r#"{"u":1,"s":"SOLUSDT","b":"100.0","B":"1","a":"102.0","A":"1"}"#;
        super::handle_frame(msg, &state);
        let guard = state.read().expect("read");
        let cp = guard.as_ref().expect("some");
        assert!((cp.price - 101.0).abs() < 1e-9);
    }

    #[test]
    fn handle_frame_ignores_malformed_and_keeps_state() {
        let state: super::CexPriceState = Arc::new(RwLock::new(None));
        super::handle_frame("garbage", &state);
        assert!(state.read().expect("read").is_none());
    }
}
