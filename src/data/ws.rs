use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};

const PING_INTERVAL: Duration = Duration::from_secs(30);
const PONG_TIMEOUT: Duration = Duration::from_secs(10);
const RECONNECT_BASE: Duration = Duration::from_secs(1);
const RECONNECT_MAX: Duration = Duration::from_secs(30);

/// Callback invoked for each `accountNotification` JSON value received.
pub type NotifyFn = Box<dyn Fn(serde_json::Value) + Send + 'static>;

/// Watch a Solana account via WebSocket with exponential-backoff reconnect,
/// periodic ping/pong keepalive, and graceful shutdown.
///
/// * `ws_url`      — WebSocket endpoint (e.g. `wss://api.devnet.solana.com`)
/// * `account`     — Base-58 account address to subscribe to
/// * `shutdown`    — Broadcast receiver; send any value to stop the loop
/// * `on_notify`   — Called for every `accountNotification` message
pub async fn watch_account(
    ws_url: String,
    account: String,
    mut shutdown: broadcast::Receiver<()>,
    on_notify: NotifyFn,
) {
    let mut backoff = RECONNECT_BASE;

    loop {
        // Respect shutdown before each reconnect attempt.
        if shutdown.try_recv().is_ok() {
            info!("WS watch: shutdown received, exiting");
            return;
        }

        info!("WS watch: connecting to {}", ws_url);
        match run_session(&ws_url, &account, &mut shutdown, &on_notify).await {
            SessionResult::Shutdown => {
                info!("WS watch: clean shutdown");
                return;
            }
            SessionResult::Reconnect { reason, connected } => {
                if connected {
                    // The WS handshake succeeded; session dropped later.
                    // Reset so flapping connections don't saturate at 30 s.
                    backoff = RECONNECT_BASE;
                }
                warn!("WS watch: session ended ({}), reconnecting in {:?}", reason, backoff);
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown.recv() => {
                        info!("WS watch: shutdown during backoff");
                        return;
                    }
                }
                backoff = (backoff * 2).min(RECONNECT_MAX);
            }
        }
    }
}

enum SessionResult {
    Shutdown,
    /// Session ended and the caller should reconnect.
    ///
    /// `connected: true`  — the WebSocket handshake itself succeeded; the
    ///                       session ended later (subscribe error, ping timeout,
    ///                       stream closed, etc.).  Backoff should reset to base.
    /// `connected: false` — `connect_async` failed immediately.  Backoff should
    ///                       keep growing.
    Reconnect { reason: String, connected: bool },
}

async fn run_session(
    ws_url: &str,
    account: &str,
    shutdown: &mut broadcast::Receiver<()>,
    on_notify: &NotifyFn,
) -> SessionResult {
    let (ws_stream, _) = match connect_async(ws_url).await {
        Ok(pair) => {
            // Successful connect resets backoff — caller resets after we return.
            pair
        }
        Err(e) => return SessionResult::Reconnect { reason: format!("connect error: {e}"), connected: false },
    };

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to the account.
    let subscribe = serde_json::json!({
        "jsonrpc": "2.0", "id": 1,
        "method": "accountSubscribe",
        "params": [account, {"encoding": "base64", "commitment": "confirmed"}]
    });
    if let Err(e) = write.send(Message::Text(subscribe.to_string())).await {
        return SessionResult::Reconnect { reason: format!("subscribe send error: {e}"), connected: true };
    }

    info!("WS watch: subscribed to {}", account);

    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.tick().await; // consume the immediate first tick

    // Track whether we are waiting for a pong.
    let mut pong_deadline: Option<tokio::time::Instant> = None;

    loop {
        // Check pong timeout.
        if let Some(deadline) = pong_deadline {
            if tokio::time::Instant::now() >= deadline {
                return SessionResult::Reconnect { reason: "pong timeout".to_string(), connected: true };
            }
        }

        tokio::select! {
            biased;

            // Shutdown signal.
            _ = shutdown.recv() => {
                return SessionResult::Shutdown;
            }

            // Periodic ping.
            _ = ping_interval.tick() => {
                if let Err(e) = write.send(Message::Ping(vec![])).await {
                    return SessionResult::Reconnect { reason: format!("ping send error: {e}"), connected: true };
                }
                pong_deadline = Some(tokio::time::Instant::now() + PONG_TIMEOUT);
            }

            // Incoming WS message.
            msg = read.next() => {
                match msg {
                    None => {
                        return SessionResult::Reconnect { reason: "stream closed".to_string(), connected: true };
                    }
                    Some(Err(e)) => {
                        warn!("WS watch: message error: {}", e);
                        // Error tolerance: log and reconnect rather than panic.
                        return SessionResult::Reconnect { reason: format!("message error: {e}"), connected: true };
                    }
                    Some(Ok(Message::Pong(_))) => {
                        pong_deadline = None;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        // Server-initiated ping — reply with pong.
                        if let Err(e) = write.send(Message::Pong(data)).await {
                            return SessionResult::Reconnect { reason: format!("pong send error: {e}"), connected: true };
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        return SessionResult::Reconnect { reason: "server closed connection".to_string(), connected: true };
                    }
                    Some(Ok(Message::Text(text))) => {
                        handle_text(&text, on_notify);
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        // Try treating binary as UTF-8 text; skip if it isn't.
                        match std::str::from_utf8(&bytes) {
                            Ok(text) => handle_text(text, on_notify),
                            Err(_) => warn!("WS watch: ignoring non-UTF-8 binary frame"),
                        }
                    }
                    Some(Ok(_)) => {
                        // Unknown frame variant — skip silently.
                    }
                }
            }
        }
    }
}

/// Parse a text frame and invoke the callback if it is an `accountNotification`.
/// All errors are logged and swallowed — never propagate out of the message loop.
fn handle_text(text: &str, on_notify: &NotifyFn) {
    let json: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            warn!("WS watch: failed to parse JSON ({}), skipping frame", e);
            return;
        }
    };

    if json.get("method").and_then(|v| v.as_str()) != Some("accountNotification") {
        return;
    }

    on_notify(json);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_text_ignores_malformed_json() {
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = called.clone();
        let cb: NotifyFn = Box::new(move |_| {
            called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        // Should not panic or call callback.
        handle_text("not json at all {{{", &cb);
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn handle_text_ignores_non_notification_method() {
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = called.clone();
        let cb: NotifyFn = Box::new(move |_| {
            called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        let msg = r#"{"jsonrpc":"2.0","id":1,"result":42}"#;
        handle_text(msg, &cb);
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn handle_text_calls_callback_for_notification() {
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = called.clone();
        let cb: NotifyFn = Box::new(move |_| {
            called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        let msg = r#"{"jsonrpc":"2.0","method":"accountNotification","params":{}}"#;
        handle_text(msg, &cb);
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// Verify that a successful connection (`connected: true`) resets backoff to
    /// base, while a failed connect (`connected: false`) keeps growing it.
    ///
    /// This is a pure logic test — no actual WebSocket is involved.
    #[test]
    fn backoff_resets_after_successful_connect_and_grows_after_connect_fail() {
        // Simulate the backoff logic from watch_account.
        let mut backoff = RECONNECT_BASE;

        // First attempt: connect_async fails → backoff grows.
        let result = SessionResult::Reconnect { reason: "connect error: refused".into(), connected: false };
        if let SessionResult::Reconnect { connected, .. } = result {
            if connected {
                backoff = RECONNECT_BASE;
            }
            // after sleep we would double:
            backoff = (backoff * 2).min(RECONNECT_MAX);
        }
        assert_eq!(backoff, Duration::from_secs(2), "backoff should double after connect fail");

        // Double again (another connect fail):
        let result = SessionResult::Reconnect { reason: "connect error: refused".into(), connected: false };
        if let SessionResult::Reconnect { connected, .. } = result {
            if connected {
                backoff = RECONNECT_BASE;
            }
            backoff = (backoff * 2).min(RECONNECT_MAX);
        }
        assert_eq!(backoff, Duration::from_secs(4));

        // Now a session that connected but then dropped → backoff resets to base
        // before sleeping, so next sleep is 1 s, then doubles to 2 s.
        let result = SessionResult::Reconnect { reason: "stream closed".into(), connected: true };
        if let SessionResult::Reconnect { connected, .. } = result {
            if connected {
                backoff = RECONNECT_BASE;
            }
            backoff = (backoff * 2).min(RECONNECT_MAX);
        }
        // reset to 1 s, then doubled → 2 s
        assert_eq!(backoff, Duration::from_secs(2), "backoff should reset and then double once");

        // Saturates at RECONNECT_MAX regardless of path.
        backoff = RECONNECT_MAX;
        let result = SessionResult::Reconnect { reason: "connect error: refused".into(), connected: false };
        if let SessionResult::Reconnect { connected, .. } = result {
            if connected {
                backoff = RECONNECT_BASE;
            }
            backoff = (backoff * 2).min(RECONNECT_MAX);
        }
        assert_eq!(backoff, RECONNECT_MAX, "backoff must not exceed RECONNECT_MAX");
    }
}
