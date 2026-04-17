use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

// These constants and helper are used in watch_binance_price / run_binance_session
// (same file, added in the next task). Allow dead_code during incremental build.
#[allow(dead_code)]
pub(crate) const RECONNECT_BASE: Duration = Duration::from_secs(1);
#[allow(dead_code)]
pub(crate) const RECONNECT_MAX: Duration = Duration::from_secs(30);

pub struct CexPrice {
    pub price: f64,
    pub updated_at: Instant,
}

pub type CexPriceState = Arc<RwLock<Option<CexPrice>>>;

#[allow(dead_code)]
pub(crate) fn parse_book_ticker(text: &str) -> Option<f64> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let bid: f64 = v["b"].as_str()?.parse().ok()?;
    let ask: f64 = v["a"].as_str()?.parse().ok()?;
    Some((bid + ask) / 2.0)
}

#[cfg(test)]
mod tests {
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
        let p = super::CexPrice { price: 10.0, updated_at: std::time::Instant::now() };
        assert_eq!(p.price, 10.0);
        let _ = p.updated_at;
    }
}
