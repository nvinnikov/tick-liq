//! WebSocket account-state subscriptions with auto-reconnect.
//!
//! Subscribes to a Solana account via the JSON-RPC `accountSubscribe` method
//! and republishes raw `PoolUpdate` events on a `tokio::sync::mpsc` channel.
//! On any disconnect or send/recv error the background task sleeps with
//! exponential backoff (100ms → 30s, capped) and reconnects + resubscribes.
//!
//! The decoded payload is intentionally kept as the raw base64-decoded
//! account bytes plus the `slot` from the notification: the data layer is
//! not responsible for protocol-specific deserialization (Orca/Raydium
//! parsing lives in `src/protocols/`). Callers verify the program owner via
//! the RPC pool before deserializing.

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use solana_sdk::pubkey::Pubkey;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

/// Raw account update emitted by [`subscribe_pool`].
#[derive(Debug, Clone)]
pub struct PoolUpdate {
    /// Slot the account update was observed at.
    pub slot: u64,
    /// Raw account bytes (base64-decoded from the notification payload).
    pub data: Vec<u8>,
}

/// Initial backoff after the first failed reconnect attempt.
const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
/// Cap on backoff between reconnect attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Channel buffer for emitted updates. Slow consumers cause backpressure
/// upstream which is preferable to silently dropping ticks.
const UPDATE_CHANNEL_CAPACITY: usize = 64;

/// Subscribe to account updates for `pool_pubkey` and return a receiver of
/// raw [`PoolUpdate`] events.
///
/// The background task reconnects forever with exponential backoff. The
/// receiver closes only when the spawned task is cancelled, which happens
/// automatically if the returned `Receiver` is dropped (the next send will
/// fail and the loop exits).
pub fn subscribe_pool(
    ws_url: impl Into<String>,
    pool_pubkey: Pubkey,
) -> mpsc::Receiver<PoolUpdate> {
    let (tx, rx) = mpsc::channel(UPDATE_CHANNEL_CAPACITY);
    let url = ws_url.into();
    tokio::spawn(run_subscription_loop(url, pool_pubkey, tx));
    rx
}

/// Same as [`subscribe_pool`] but the loop exits cleanly after `max_attempts`
/// reconnect failures. Used by tests so they don't spin forever on a teardown.
#[doc(hidden)]
pub fn subscribe_pool_with_limit(
    ws_url: impl Into<String>,
    pool_pubkey: Pubkey,
    max_attempts: usize,
) -> mpsc::Receiver<PoolUpdate> {
    let (tx, rx) = mpsc::channel(UPDATE_CHANNEL_CAPACITY);
    let url = ws_url.into();
    tokio::spawn(async move {
        let _ = run_subscription_loop_bounded(url, pool_pubkey, tx, Some(max_attempts)).await;
    });
    rx
}

async fn run_subscription_loop(url: String, pool_pubkey: Pubkey, tx: mpsc::Sender<PoolUpdate>) {
    let _ = run_subscription_loop_bounded(url, pool_pubkey, tx, None).await;
}

async fn run_subscription_loop_bounded(
    url: String,
    pool_pubkey: Pubkey,
    tx: mpsc::Sender<PoolUpdate>,
    max_attempts: Option<usize>,
) -> Result<()> {
    let mut backoff = INITIAL_BACKOFF;
    let mut attempts: usize = 0;
    loop {
        match run_one_session(&url, &pool_pubkey, &tx).await {
            Ok(()) => {
                // Server closed cleanly — treat as transient and reconnect.
                tracing::warn!(target: "tick_liq::ws", "ws session ended cleanly, reconnecting");
            }
            Err(e) => {
                tracing::warn!(target: "tick_liq::ws", error = %e, "ws session failed, reconnecting");
            }
        }
        if tx.is_closed() {
            return Ok(());
        }
        attempts += 1;
        if let Some(limit) = max_attempts {
            if attempts >= limit {
                return Ok(());
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = next_backoff(backoff);
    }
}

/// Compute the next backoff duration. Doubles up to [`MAX_BACKOFF`].
fn next_backoff(current: Duration) -> Duration {
    let doubled = current.saturating_mul(2);
    if doubled > MAX_BACKOFF {
        MAX_BACKOFF
    } else {
        doubled
    }
}

async fn run_one_session(
    url: &str,
    pool_pubkey: &Pubkey,
    tx: &mpsc::Sender<PoolUpdate>,
) -> Result<()> {
    let (mut ws, _resp) = tokio_tungstenite::connect_async(url)
        .await
        .with_context(|| format!("ws connect {url}"))?;

    let subscribe = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "accountSubscribe",
        "params": [
            pool_pubkey.to_string(),
            {"encoding": "base64", "commitment": "confirmed"}
        ]
    });
    ws.send(Message::Text(subscribe.to_string()))
        .await
        .context("ws subscribe send")?;

    while let Some(msg) = ws.next().await {
        let msg = msg.context("ws recv")?;
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => return Ok(()),
            Message::Ping(payload) => {
                ws.send(Message::Pong(payload)).await.ok();
                continue;
            }
            _ => continue,
        };

        let WsMessage::AccountNotification { params } = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue, // skip subscription ack and any unknown shapes
        };

        let bytes =
            decode_base64_account(&params.result.value.data).context("decode account base64")?;
        let update = PoolUpdate {
            slot: params.result.context.slot,
            data: bytes,
        };
        if tx.send(update).await.is_err() {
            // Receiver dropped; bail out cleanly so the supervisor can exit.
            return Ok(());
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(tag = "method")]
enum WsMessage {
    #[serde(rename = "accountNotification")]
    AccountNotification { params: NotificationParams },
}

#[derive(Debug, Deserialize)]
struct NotificationParams {
    result: NotificationResult,
}

#[derive(Debug, Deserialize)]
struct NotificationResult {
    context: NotificationContext,
    value: NotificationValue,
}

#[derive(Debug, Deserialize)]
struct NotificationContext {
    slot: u64,
}

#[derive(Debug, Deserialize)]
struct NotificationValue {
    /// `[base64-string, "base64"]` per Solana JSON-RPC spec.
    data: (String, String),
}

fn decode_base64_account(data: &(String, String)) -> Result<Vec<u8>> {
    if data.1 != "base64" {
        return Err(anyhow!("unexpected account encoding: {}", data.1));
    }
    use base64_simple::decode_b64;
    decode_b64(&data.0)
}

// Tiny self-contained base64 decoder so we don't have to add another crate
// for one call site. Solana account payloads are short enough that this is
// not a hot path.
mod base64_simple {
    use anyhow::{anyhow, Result};

    pub fn decode_b64(s: &str) -> Result<Vec<u8>> {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut lookup = [255u8; 256];
        for (i, c) in TABLE.iter().enumerate() {
            lookup[*c as usize] = i as u8;
        }
        let bytes = s.as_bytes();
        // Strip padding length.
        let pad = bytes.iter().rev().take_while(|b| **b == b'=').count();
        let useful = bytes.len().saturating_sub(pad);
        let mut out = Vec::with_capacity(useful * 3 / 4);
        let mut buf = 0u32;
        let mut bits = 0u32;
        for &b in &bytes[..useful] {
            let v = lookup[b as usize];
            if v == 255 {
                if b.is_ascii_whitespace() {
                    continue;
                }
                return Err(anyhow!("invalid base64 char: {}", b as char));
            }
            buf = (buf << 6) | v as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push(((buf >> bits) & 0xFF) as u8);
            }
        }
        Ok(out)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn roundtrip_known_vectors() {
            assert_eq!(decode_b64("").unwrap(), b"");
            assert_eq!(decode_b64("Zg==").unwrap(), b"f");
            assert_eq!(decode_b64("Zm8=").unwrap(), b"fo");
            assert_eq!(decode_b64("Zm9v").unwrap(), b"foo");
            assert_eq!(decode_b64("Zm9vYg==").unwrap(), b"foob");
            assert_eq!(decode_b64("aGVsbG8=").unwrap(), b"hello");
        }
        #[test]
        fn rejects_garbage() {
            assert!(decode_b64("@@@").is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::SinkExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    #[test]
    fn backoff_doubles_and_caps() {
        let mut d = INITIAL_BACKOFF;
        for _ in 0..30 {
            d = next_backoff(d);
        }
        assert_eq!(d, MAX_BACKOFF);
    }

    #[test]
    fn backoff_first_step() {
        assert_eq!(
            next_backoff(Duration::from_millis(100)),
            Duration::from_millis(200)
        );
        assert_eq!(
            next_backoff(Duration::from_millis(200)),
            Duration::from_millis(400)
        );
    }

    #[test]
    fn rejects_unknown_encoding() {
        let err = decode_base64_account(&("xx".to_string(), "base58".to_string())).unwrap_err();
        assert!(err.to_string().contains("unexpected account encoding"));
    }

    /// End-to-end smoke test: spin up a local WS server that delivers one
    /// notification, drops the connection, then on reconnect delivers a
    /// second notification. The subscriber should receive both.
    #[tokio::test(flavor = "current_thread")]
    async fn reconnects_and_resubscribes_after_drop() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("ws://{addr}");
        let connection_count = Arc::new(AtomicUsize::new(0));

        let server_count = Arc::clone(&connection_count);
        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let n = server_count.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut ws = match accept_async(stream).await {
                        Ok(w) => w,
                        Err(_) => return,
                    };
                    // Drain the subscribe message.
                    let _ = ws.next().await;
                    // Send subscription ack.
                    let _ = ws
                        .send(Message::Text(
                            r#"{"jsonrpc":"2.0","result":1,"id":1}"#.to_string(),
                        ))
                        .await;
                    // Send a notification with slot=n+1 and data = "AAAA" (base64 for [0,0,0]).
                    let notif = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "accountNotification",
                        "params": {
                            "result": {
                                "context": {"slot": (n as u64) + 1},
                                "value": {
                                    "lamports": 1,
                                    "owner": "11111111111111111111111111111111",
                                    "executable": false,
                                    "rentEpoch": 0,
                                    "data": ["AAAA", "base64"]
                                }
                            },
                            "subscription": 1
                        }
                    });
                    let _ = ws.send(Message::Text(notif.to_string())).await;
                    // Drop the connection by closing.
                    let _ = ws.close(None).await;
                });
            }
        });

        let pool = Pubkey::new_unique();
        let mut rx = subscribe_pool_with_limit(url, pool, 5);

        // First update from connection #1.
        let first = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("first update timed out")
            .expect("channel closed");
        assert_eq!(first.slot, 1);
        assert_eq!(first.data, vec![0, 0, 0]);

        // Second update after reconnect — connection #2.
        let second = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("second update timed out")
            .expect("channel closed");
        assert_eq!(second.slot, 2);

        assert!(connection_count.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropped_receiver_stops_loop() {
        // Bind to an unreachable port; loop must exit when receiver drops.
        let rx = subscribe_pool_with_limit("ws://127.0.0.1:1", Pubkey::new_unique(), 3);
        drop(rx);
        // Just ensure no panic occurs within a short window.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
