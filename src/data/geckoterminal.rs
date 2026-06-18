//! GeckoTerminal REST client for historical pool OHLCV.
//!
//! Free, key-less public API. Used by the `backfill` command to pull months of
//! real price + volume history for a Solana CLMM pool and seed `pool_ticks`, so
//! the DB-replay backtest can run on real data instead of a synthetic GBM path.
//!
//! Docs: <https://www.geckoterminal.com/dex-api>. Endpoint shape:
//! `GET /api/v2/networks/solana/pools/{pool}/ohlcv/{timeframe}?limit=1000&currency=usd&before_timestamp={ts}`
//! → `data.attributes.ohlcv_list = [[ts, open, high, low, close, volume_usd], …]`
//! returned newest-first. Volume is in USD (`currency=usd`).

use anyhow::{Context, Result, bail};
use serde_json::Value;

const BASE_URL: &str = "https://api.geckoterminal.com/api/v2";
/// GeckoTerminal caps OHLCV responses at 1000 candles per request.
const MAX_LIMIT: u32 = 1000;

/// One OHLCV candle. `volume_usd` is the pool's traded volume over the period.
#[derive(Debug, Clone, PartialEq)]
pub struct OhlcvCandle {
    pub timestamp: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume_usd: f64,
}

/// Parse a GeckoTerminal OHLCV response body into candles sorted **ascending**
/// by timestamp. Pure — no I/O — so it is unit-tested against a captured fixture.
///
/// Each `ohlcv_list` entry is `[timestamp, open, high, low, close, volume]`.
/// Malformed rows (wrong arity / non-numeric) cause an error rather than a
/// silently dropped candle, so data gaps surface instead of skewing fees.
pub fn parse_ohlcv_list(body: &Value) -> Result<Vec<OhlcvCandle>> {
    let list = body
        .pointer("/data/attributes/ohlcv_list")
        .and_then(Value::as_array)
        .context("response missing data.attributes.ohlcv_list array")?;

    let mut out = Vec::with_capacity(list.len());
    for (i, entry) in list.iter().enumerate() {
        let row = entry
            .as_array()
            .with_context(|| format!("ohlcv_list[{i}] is not an array"))?;
        if row.len() != 6 {
            bail!("ohlcv_list[{i}] has {} fields, expected 6", row.len());
        }
        let num = |j: usize, name: &str| -> Result<f64> {
            row[j]
                .as_f64()
                .with_context(|| format!("ohlcv_list[{i}].{name} is not a number"))
        };
        out.push(OhlcvCandle {
            timestamp: num(0, "timestamp")? as i64,
            open: num(1, "open")?,
            high: num(2, "high")?,
            low: num(3, "low")?,
            close: num(4, "close")?,
            volume_usd: num(5, "volume")?,
        });
    }

    out.sort_by_key(|c| c.timestamp);
    Ok(out)
}

/// Fetch a single page of OHLCV candles (newest-first window, optionally ending
/// before `before_timestamp`). Returns candles sorted ascending.
pub async fn fetch_ohlcv(
    client: &reqwest::Client,
    pool_address: &str,
    timeframe: &str,
    before_timestamp: Option<i64>,
    limit: u32,
) -> Result<Vec<OhlcvCandle>> {
    let mut url = format!(
        "{BASE_URL}/networks/solana/pools/{pool_address}/ohlcv/{timeframe}\
         ?limit={}&currency=usd",
        limit.min(MAX_LIMIT)
    );
    if let Some(ts) = before_timestamp {
        url.push_str(&format!("&before_timestamp={ts}"));
    }

    let resp = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .context("GeckoTerminal request failed")?;

    let status = resp.status();
    if !status.is_success() {
        bail!("GeckoTerminal returned HTTP {status} for pool {pool_address}");
    }

    let body: Value = resp.json().await.context("decode GeckoTerminal JSON")?;
    parse_ohlcv_list(&body)
}

/// Fetch all candles covering `[from_ts, to_ts)`, paginating backwards with
/// `before_timestamp` until the window is covered or the API runs dry.
///
/// `from_ts` / `to_ts` are UTC unix seconds. Returns candles in `[from_ts, to_ts)`
/// sorted ascending. Caps total pages to avoid an unbounded loop on a misbehaving
/// endpoint (logged, not silent).
pub async fn fetch_range(
    client: &reqwest::Client,
    pool_address: &str,
    timeframe: &str,
    from_ts: i64,
    to_ts: i64,
) -> Result<Vec<OhlcvCandle>> {
    const MAX_PAGES: usize = 50; // 50 * 1000 candles is far beyond any sane range

    let mut all: Vec<OhlcvCandle> = Vec::new();
    let mut cursor = Some(to_ts);

    for page in 0..MAX_PAGES {
        let batch = fetch_ohlcv(client, pool_address, timeframe, cursor, MAX_LIMIT).await?;
        if batch.is_empty() {
            break;
        }
        let oldest = batch.first().map(|c| c.timestamp).unwrap_or(from_ts);
        all.extend(batch);

        if oldest <= from_ts {
            break; // reached the start of the requested window
        }
        // Next page ends just before the oldest candle we just saw.
        cursor = Some(oldest - 1);

        if page == MAX_PAGES - 1 {
            tracing::warn!(
                pool = pool_address,
                "fetch_range hit MAX_PAGES; history may be truncated before {from_ts}"
            );
        }
    }

    all.retain(|c| c.timestamp >= from_ts && c.timestamp < to_ts);
    all.sort_by_key(|c| c.timestamp);
    all.dedup_by_key(|c| c.timestamp);
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from the live API (Orca SOL/USDC pool), trimmed to 3 candles.
    // Note: newest-first in the wire format — the parser must sort ascending.
    const FIXTURE: &str = r#"{
        "data": {
            "id": "abc",
            "type": "ohlcv_request_response",
            "attributes": {
                "ohlcv_list": [
                    [1781740800, 72.05, 72.66, 68.16, 69.58, 96380536.03],
                    [1781654400, 73.48, 74.47, 70.86, 72.05, 155823217.43],
                    [1781568000, 73.87, 75.62, 72.26, 73.48, 100163397.64]
                ]
            }
        }
    }"#;

    #[test]
    fn parses_and_sorts_ascending() {
        let v: Value = serde_json::from_str(FIXTURE).unwrap();
        let candles = parse_ohlcv_list(&v).unwrap();
        assert_eq!(candles.len(), 3);
        // Wire order is newest-first; output must be oldest-first.
        assert_eq!(candles[0].timestamp, 1781568000);
        assert_eq!(candles[2].timestamp, 1781740800);
        assert!((candles[0].close - 73.48).abs() < 1e-9);
        assert!((candles[2].volume_usd - 96380536.03).abs() < 1e-2);
    }

    #[test]
    fn missing_list_errors() {
        let v: Value = serde_json::json!({"data": {"attributes": {}}});
        assert!(parse_ohlcv_list(&v).is_err());
    }

    #[test]
    fn wrong_arity_row_errors() {
        let v: Value = serde_json::json!({
            "data": {"attributes": {"ohlcv_list": [[1, 2, 3]]}}
        });
        assert!(parse_ohlcv_list(&v).is_err());
    }

    #[test]
    fn empty_list_is_ok_empty() {
        let v: Value = serde_json::json!({
            "data": {"attributes": {"ohlcv_list": []}}
        });
        assert_eq!(parse_ohlcv_list(&v).unwrap().len(), 0);
    }
}
