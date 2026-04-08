//! Price feed adapters: Pyth on-chain + CEX HTTP poll, with a composite
//! fallback source.
//!
//! ## Shape
//!
//! - [`PriceTick`] — the common output: `price`, `conf`, `ts` (UNIX seconds).
//! - [`PriceFeed`] — async trait implemented by each source.
//! - [`PythFeed`] — reads a Pyth V2 price account via the RPC pool, decodes
//!   the aggregate price at fixed offsets (no `pyth-sdk` dep to keep build
//!   graph small).
//! - [`CexFeed`] — polls Binance's public spot ticker REST endpoint via
//!   `reqwest`. A custom base URL can be injected for tests.
//! - [`CompositeFeed`] — prefers Pyth; if the last Pyth tick is older than
//!   `staleness_threshold`, falls back to the CEX feed.
//!
//! `PythFeed` verifies that the fetched account is owned by the configured
//! Pyth program id on every fetch (via [`RpcPool::verify_owner`]) before
//! decoding — this closes the spoofing hole left open by earlier versions
//! where the decoder trusted whatever bytes came back. Callers that don't
//! have a program id at construction time can use
//! [`PythFeed::new_unchecked`] to opt out, but production call sites should
//! always pass one.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use std::time::Duration;

use crate::data::rpc::{AccountSnapshot, RpcPool};

/// A single price observation from any source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriceTick {
    /// Price in USD (already adjusted for Pyth `expo` when from Pyth).
    pub price: f64,
    /// Confidence interval in USD. For CEX feeds this is 0.0 (no native
    /// concept); callers that need it should treat zero as "unknown".
    pub conf: f64,
    /// UNIX seconds timestamp the tick was produced at (publish time from
    /// Pyth, wall clock at fetch time from CEX).
    pub ts: i64,
}

/// Async trait implemented by each price source. Designed to be trait-object
/// safe so the composite feed can hold `Arc<dyn PriceFeed>`.
#[async_trait]
pub trait PriceFeed: Send + Sync {
    async fn fetch(&self) -> Result<PriceTick>;
}

// -----------------------------------------------------------------------------
// Pyth feed
// -----------------------------------------------------------------------------

/// Pyth V2 price-account reader.
///
/// The byte layout we read is the stable subset of the Pyth V2 price account:
/// <https://github.com/pyth-network/pyth-client/blob/main/program/rust/src/accounts/price.rs>
///
/// We only care about the *aggregate* price (the smoothed value Pyth
/// publishes), not the individual publisher components. The relevant fields
/// are at fixed offsets inside the aggregate sub-struct:
///
/// ```text
/// offset  type   field
/// ------  -----  -------------------------
///     0   u32    magic                    = 0xa1b2c3d4
///     4   u32    version
///     8   u32    account_type             (3 = Price)
///    12   u32    size
///    16   i32    price_type
///    20   i32    expo
///    // ... other header fields ...
///   208   i64    agg.price
///   216   u64    agg.conf
///   // ... publish slot, prev_*, etc ...
///   240   u64    agg.pub_slot
///   // ... then the timestamp sub-struct:
///   280   i64    timestamp (publish_time)
/// ```
///
/// Offsets verified against Pyth client v2.27 on mainnet and used by several
/// independent downstream consumers; if Pyth ships a V3 account we'll need
/// to adapt, which is tracked as a future task.
pub struct PythFeed {
    rpc: RpcPool,
    price_pubkey: Pubkey,
    /// Expected program owner of the price account. When `Some`, every
    /// fetch verifies `account.owner == expected_owner` before decoding.
    expected_owner: Option<Pubkey>,
}

impl PythFeed {
    /// Build a feed that verifies the price account is owned by
    /// `pyth_program_id` on every fetch. This is the recommended
    /// constructor for all production call sites.
    pub fn new(rpc: RpcPool, price_pubkey: Pubkey, pyth_program_id: Pubkey) -> Self {
        Self {
            rpc,
            price_pubkey,
            expected_owner: Some(pyth_program_id),
        }
    }

    /// Build a feed that skips the program-owner check. Only for contexts
    /// where the owner cannot be known at construction time (e.g. ad-hoc
    /// tooling or tests). Production paths should prefer [`PythFeed::new`].
    pub fn new_unchecked(rpc: RpcPool, price_pubkey: Pubkey) -> Self {
        Self {
            rpc,
            price_pubkey,
            expected_owner: None,
        }
    }

    /// Verify + decode a pre-fetched snapshot. Extracted so tests can
    /// exercise the owner-check path without standing up a live RPC.
    fn decode_snapshot(&self, snap: &AccountSnapshot) -> Result<PriceTick> {
        let bytes = match &self.expected_owner {
            Some(owner) => RpcPool::verify_owner(snap, owner)
                .with_context(|| format!("pyth price account {} owner check", self.price_pubkey))?,
            None => snap.data.as_slice(),
        };
        decode_pyth_price(bytes)
    }
}

const PYTH_MAGIC: u32 = 0xa1b2_c3d4;
const PYTH_OFFSET_EXPO: usize = 20;
const PYTH_OFFSET_AGG_PRICE: usize = 208;
const PYTH_OFFSET_AGG_CONF: usize = 216;
const PYTH_OFFSET_TIMESTAMP: usize = 280;
/// Minimum length required to decode all fields we use (timestamp + i64).
const PYTH_MIN_LEN: usize = PYTH_OFFSET_TIMESTAMP + 8;

/// Decode a Pyth V2 price account payload into a [`PriceTick`].
///
/// Public only so the test suite in this file can fabricate a valid payload.
/// External callers should go through [`PythFeed::fetch`].
pub fn decode_pyth_price(bytes: &[u8]) -> Result<PriceTick> {
    if bytes.len() < PYTH_MIN_LEN {
        return Err(anyhow!(
            "pyth account too short: {} < {}",
            bytes.len(),
            PYTH_MIN_LEN
        ));
    }
    let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if magic != PYTH_MAGIC {
        return Err(anyhow!("pyth magic mismatch: got 0x{magic:08x}"));
    }
    let expo = i32::from_le_bytes(
        bytes[PYTH_OFFSET_EXPO..PYTH_OFFSET_EXPO + 4]
            .try_into()
            .unwrap(),
    );
    let raw_price = i64::from_le_bytes(
        bytes[PYTH_OFFSET_AGG_PRICE..PYTH_OFFSET_AGG_PRICE + 8]
            .try_into()
            .unwrap(),
    );
    let raw_conf = u64::from_le_bytes(
        bytes[PYTH_OFFSET_AGG_CONF..PYTH_OFFSET_AGG_CONF + 8]
            .try_into()
            .unwrap(),
    );
    let ts = i64::from_le_bytes(
        bytes[PYTH_OFFSET_TIMESTAMP..PYTH_OFFSET_TIMESTAMP + 8]
            .try_into()
            .unwrap(),
    );

    // Pyth stores price * 10^expo; expo is typically negative (e.g. -8).
    let scale = 10f64.powi(expo);
    Ok(PriceTick {
        price: raw_price as f64 * scale,
        conf: raw_conf as f64 * scale,
        ts,
    })
}

#[async_trait]
impl PriceFeed for PythFeed {
    async fn fetch(&self) -> Result<PriceTick> {
        let snap = self
            .rpc
            .fetch_account_data(&self.price_pubkey)
            .await
            .with_context(|| format!("fetch pyth account {}", self.price_pubkey))?;
        self.decode_snapshot(&snap)
    }
}

// -----------------------------------------------------------------------------
// CEX feed (Binance ticker)
// -----------------------------------------------------------------------------

/// HTTP price poller for Binance spot ticker.
///
/// Uses `/api/v3/ticker/price?symbol=...` which returns
/// `{"symbol":"SOLUSDT","price":"142.57"}`. A custom `base_url` may be
/// injected so tests can point at a wiremock instance.
pub struct CexFeed {
    client: reqwest::Client,
    base_url: String,
    symbol: String,
}

impl CexFeed {
    /// Build a feed pointing at Binance public REST.
    pub fn binance(symbol: impl Into<String>) -> Self {
        Self::with_base_url("https://api.binance.com", symbol)
    }

    /// Build a feed pointing at an arbitrary base URL. Intended for tests.
    pub fn with_base_url(base_url: impl Into<String>, symbol: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("reqwest client build");
        Self {
            client,
            base_url: base_url.into(),
            symbol: symbol.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct BinanceTickerResponse {
    #[serde(rename = "price")]
    price: String,
}

#[async_trait]
impl PriceFeed for CexFeed {
    async fn fetch(&self) -> Result<PriceTick> {
        let url = format!(
            "{}/api/v3/ticker/price?symbol={}",
            self.base_url, self.symbol
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("non-2xx from {url}"))?;
        let body: BinanceTickerResponse = resp
            .json()
            .await
            .with_context(|| format!("decode ticker body from {url}"))?;
        let price: f64 = body
            .price
            .parse()
            .with_context(|| format!("parse price '{}' as f64", body.price))?;
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Ok(PriceTick {
            price,
            conf: 0.0,
            ts,
        })
    }
}

// -----------------------------------------------------------------------------
// Composite feed
// -----------------------------------------------------------------------------

/// Composite feed: prefers `primary`, falls back to `fallback` when the
/// primary's most recent tick is older than `staleness_threshold` seconds or
/// the primary returns an error.
///
/// The intended pairing is `primary = PythFeed`, `fallback = CexFeed` so
/// that an oracle outage doesn't stall the strategy layer.
pub struct CompositeFeed {
    primary: Arc<dyn PriceFeed>,
    fallback: Arc<dyn PriceFeed>,
    staleness_threshold: Duration,
    now: fn() -> i64,
}

impl CompositeFeed {
    pub fn new(
        primary: Arc<dyn PriceFeed>,
        fallback: Arc<dyn PriceFeed>,
        staleness_threshold: Duration,
    ) -> Self {
        Self {
            primary,
            fallback,
            staleness_threshold,
            now: default_now,
        }
    }

    /// Test hook: override the wall-clock source so we can control
    /// staleness deterministically.
    #[cfg(test)]
    fn with_now(mut self, now: fn() -> i64) -> Self {
        self.now = now;
        self
    }
}

fn default_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[async_trait]
impl PriceFeed for CompositeFeed {
    async fn fetch(&self) -> Result<PriceTick> {
        match self.primary.fetch().await {
            Ok(tick) => {
                let age = (self.now)().saturating_sub(tick.ts);
                if age <= self.staleness_threshold.as_secs() as i64 {
                    return Ok(tick);
                }
                tracing::warn!(
                    target: "tick_liq::prices",
                    age_secs = age,
                    "primary feed stale, falling back"
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "tick_liq::prices",
                    error = %e,
                    "primary feed errored, falling back"
                );
            }
        }
        self.fallback.fetch().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // --- Pyth decode ---

    fn fabricate_pyth_bytes(price_raw: i64, conf_raw: u64, expo: i32, ts: i64) -> Vec<u8> {
        let mut buf = vec![0u8; PYTH_MIN_LEN];
        buf[0..4].copy_from_slice(&PYTH_MAGIC.to_le_bytes());
        buf[PYTH_OFFSET_EXPO..PYTH_OFFSET_EXPO + 4].copy_from_slice(&expo.to_le_bytes());
        buf[PYTH_OFFSET_AGG_PRICE..PYTH_OFFSET_AGG_PRICE + 8]
            .copy_from_slice(&price_raw.to_le_bytes());
        buf[PYTH_OFFSET_AGG_CONF..PYTH_OFFSET_AGG_CONF + 8]
            .copy_from_slice(&conf_raw.to_le_bytes());
        buf[PYTH_OFFSET_TIMESTAMP..PYTH_OFFSET_TIMESTAMP + 8].copy_from_slice(&ts.to_le_bytes());
        buf
    }

    #[test]
    fn pyth_decode_happy_path() {
        // price = 14257_000000, conf = 10_000000, expo = -8 -> 142.57, 0.10
        let bytes = fabricate_pyth_bytes(14_257_000_000, 10_000_000, -8, 1_700_000_000);
        let tick = decode_pyth_price(&bytes).unwrap();
        assert!((tick.price - 142.57).abs() < 1e-9, "got {}", tick.price);
        assert!((tick.conf - 0.10).abs() < 1e-9);
        assert_eq!(tick.ts, 1_700_000_000);
    }

    #[test]
    fn pyth_decode_negative_price_allowed() {
        // Some pairs (perp funding etc) can legally be negative. Decoder
        // shouldn't reject — interpretation is the caller's problem.
        let bytes = fabricate_pyth_bytes(-500_000_000, 0, -6, 0);
        let tick = decode_pyth_price(&bytes).unwrap();
        assert!((tick.price - (-500.0)).abs() < 1e-9);
    }

    #[test]
    fn pyth_decode_rejects_short_buffer() {
        let err = decode_pyth_price(&[0u8; 16]).unwrap_err();
        assert!(err.to_string().contains("too short"));
    }

    fn dummy_rpc() -> RpcPool {
        RpcPool::new("http://127.0.0.1:1")
    }

    fn snap_with_owner(data: Vec<u8>, owner: Pubkey) -> AccountSnapshot {
        AccountSnapshot {
            data,
            owner,
            lamports: 0,
        }
    }

    #[test]
    fn pyth_decode_snapshot_verifies_owner_match() {
        let bytes = fabricate_pyth_bytes(100, 0, -2, 42);
        let owner = Pubkey::new_unique();
        let feed = PythFeed::new(dummy_rpc(), Pubkey::new_unique(), owner);
        let snap = snap_with_owner(bytes, owner);
        let tick = feed.decode_snapshot(&snap).unwrap();
        assert!((tick.price - 1.0).abs() < 1e-9);
    }

    #[test]
    fn pyth_decode_snapshot_rejects_wrong_owner() {
        let bytes = fabricate_pyth_bytes(100, 0, -2, 42);
        let expected = Pubkey::new_unique();
        let actual = Pubkey::new_unique();
        let feed = PythFeed::new(dummy_rpc(), Pubkey::new_unique(), expected);
        let snap = snap_with_owner(bytes, actual);
        let err = feed.decode_snapshot(&snap).unwrap_err();
        let chain = format!("{err:#}");
        assert!(chain.contains("owner"), "got: {chain}");
    }

    #[test]
    fn pyth_decode_snapshot_unchecked_skips_owner() {
        let bytes = fabricate_pyth_bytes(100, 0, -2, 42);
        // Owner is arbitrary garbage — unchecked feed does not care.
        let feed = PythFeed::new_unchecked(dummy_rpc(), Pubkey::new_unique());
        let snap = snap_with_owner(bytes, Pubkey::new_unique());
        assert!(feed.decode_snapshot(&snap).is_ok());
    }

    #[test]
    fn pyth_decode_rejects_bad_magic() {
        let mut bytes = fabricate_pyth_bytes(1, 0, 0, 0);
        bytes[0..4].copy_from_slice(&0xdead_beefu32.to_le_bytes());
        let err = decode_pyth_price(&bytes).unwrap_err();
        assert!(err.to_string().contains("magic mismatch"));
    }

    // --- CEX feed against wiremock ---

    #[tokio::test(flavor = "current_thread")]
    async fn cex_feed_parses_binance_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/ticker/price"))
            .and(query_param("symbol", "SOLUSDT"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"symbol":"SOLUSDT","price":"142.57"}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let feed = CexFeed::with_base_url(server.uri(), "SOLUSDT");
        let tick = feed.fetch().await.unwrap();
        assert!((tick.price - 142.57).abs() < 1e-9);
        assert_eq!(tick.conf, 0.0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cex_feed_errors_on_5xx() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/ticker/price"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let feed = CexFeed::with_base_url(server.uri(), "SOLUSDT");
        assert!(feed.fetch().await.is_err());
    }

    // --- Composite feed ---

    struct StaticFeed {
        tick: PriceTick,
    }
    #[async_trait]
    impl PriceFeed for StaticFeed {
        async fn fetch(&self) -> Result<PriceTick> {
            Ok(self.tick)
        }
    }

    struct FailingFeed;
    #[async_trait]
    impl PriceFeed for FailingFeed {
        async fn fetch(&self) -> Result<PriceTick> {
            Err(anyhow!("boom"))
        }
    }

    struct CountingFeed {
        tick: PriceTick,
        calls: AtomicUsize,
    }
    #[async_trait]
    impl PriceFeed for CountingFeed {
        async fn fetch(&self) -> Result<PriceTick> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.tick)
        }
    }

    fn tick(price: f64, ts: i64) -> PriceTick {
        PriceTick {
            price,
            conf: 0.0,
            ts,
        }
    }

    fn now_1000() -> i64 {
        1000
    }

    #[tokio::test(flavor = "current_thread")]
    async fn composite_prefers_primary_when_fresh() {
        let primary = Arc::new(StaticFeed {
            tick: tick(100.0, 995),
        });
        let fallback = Arc::new(StaticFeed {
            tick: tick(200.0, 995),
        });
        let composite =
            CompositeFeed::new(primary, fallback, Duration::from_secs(10)).with_now(now_1000);
        let got = composite.fetch().await.unwrap();
        assert_eq!(got.price, 100.0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn composite_falls_back_when_primary_stale() {
        let primary = Arc::new(StaticFeed {
            tick: tick(100.0, 500),
        }); // 500s old
        let fallback = Arc::new(CountingFeed {
            tick: tick(200.0, 1000),
            calls: AtomicUsize::new(0),
        });
        let fallback_ref = Arc::clone(&fallback) as Arc<dyn PriceFeed>;
        let composite =
            CompositeFeed::new(primary, fallback_ref, Duration::from_secs(10)).with_now(now_1000);
        let got = composite.fetch().await.unwrap();
        assert_eq!(got.price, 200.0);
        assert_eq!(fallback.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn composite_falls_back_when_primary_errors() {
        let primary = Arc::new(FailingFeed);
        let fallback = Arc::new(StaticFeed {
            tick: tick(200.0, 1000),
        });
        let composite =
            CompositeFeed::new(primary, fallback, Duration::from_secs(10)).with_now(now_1000);
        let got = composite.fetch().await.unwrap();
        assert_eq!(got.price, 200.0);
    }
}
