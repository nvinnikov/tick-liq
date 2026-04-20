---
phase: 11-cex-price-feed-via-binance-websocket
plan: "01"
subsystem: data
tags: [websocket, binance, async, data-layer, tdd]
requirements: [CEX-01, CEX-02, CEX-03, CEX-05]

dependency_graph:
  requires: []
  provides:
    - src/data/cex_ws.rs (watch_binance_price, CexPrice, CexPriceState, parse_book_ticker)
  affects:
    - src/data/mod.rs (new pub mod cex_ws export)

tech_stack:
  added: []
  patterns:
    - Reconnect loop with exponential backoff (1s→30s) mirroring src/data/ws.rs
    - std::sync::RwLock for sync-safe shared state (no block_in_place needed on reader)
    - Server-ping response pattern (Binance sends pings; we pong — no client-initiated pings)
    - parse_book_ticker: serde_json + .ok()? chain; None on any malformed input

key_files:
  created:
    - src/data/cex_ws.rs
  modified:
    - src/data/mod.rs

decisions:
  - Used std::sync::RwLock (not tokio::sync::RwLock) so Plan 02 can read from sync on_notify closure without block_in_place
  - parse_book_ticker marked pub(crate) to avoid dead_code lint before Task 2 wires the call sites
  - SessionResult::Reconnect has no reason field (simplified vs ws.rs — caller logs at warn! site)
  - Message::Close returns Reconnect { connected: true } to handle Binance 24h forced disconnect as a normal event
  - RwLock poisoning recovered via unwrap_or_else(|p| p.into_inner()) per T-11-03 mitigation

metrics:
  duration_seconds: 358
  completed_date: "2026-04-17"
  tasks_completed: 2
  tasks_total: 2
  files_created: 1
  files_modified: 1
---

# Phase 11 Plan 01: Binance bookTicker WebSocket module Summary

**One-liner:** Binance `@bookTicker` WebSocket feed with exponential-backoff reconnect, mid-price parser, and `Arc<RwLock<Option<CexPrice>>>` shared state — ready for Plan 02 wiring.

## Public API Delivered

```rust
// src/data/cex_ws.rs

pub struct CexPrice {
    pub price: f64,          // mid-price = (bid + ask) / 2
    pub updated_at: Instant, // for staleness check in Plan 02
}

pub type CexPriceState = Arc<RwLock<Option<CexPrice>>>;

/// Connects to wss://stream.binance.com:9443/ws/{symbol}@bookTicker.
/// Lowercases symbol internally. Writes mid-price on every frame.
/// Auto-reconnects with exponential backoff. Returns on shutdown broadcast.
pub async fn watch_binance_price(
    symbol: String,
    state: CexPriceState,
    shutdown: broadcast::Receiver<()>,
);
```

Internal helpers (pub(crate) for visibility without dead_code lint):
- `parse_book_ticker(text: &str) -> Option<f64>`
- `build_stream_url(symbol: &str) -> String`
- `handle_frame(text: &str, state: &CexPriceState)`

## Tests Added

10 unit tests across 2 tasks:

| Test | Task | What it validates |
|------|------|-------------------|
| parse_book_ticker_valid | 1 | mid=(140.20+140.30)/2=140.25 |
| parse_book_ticker_malformed_json | 1 | returns None, no panic |
| parse_book_ticker_missing_bid_ask | 1 | returns None |
| parse_book_ticker_non_numeric | 1 | returns None for non-numeric fields |
| cex_price_struct_has_public_fields | 1 | struct compiles with accessible fields |
| build_stream_url_lowercases_symbol | 2 | SOLUSDT → solusdt in URL |
| backoff_grows_then_resets | 2 | 1s→2s→4s on failure; resets to 1s→2s on success |
| backoff_saturates_at_max | 2 | 30s stays 30s after another failure |
| handle_frame_updates_state | 2 | writes mid-price to RwLock state |
| handle_frame_ignores_malformed_and_keeps_state | 2 | state stays None on garbage input |

## Module Size

- `src/data/cex_ws.rs`: 232 lines (min_lines requirement: 120 — exceeded)

## Deviations from Plan

### Auto-fixed Issues

None — plan executed exactly as written, with one minor deviation:

**1. [Rule 2 - Visibility] parse_book_ticker / constants marked pub(crate)**
- **Found during:** Task 1 clippy verification
- **Issue:** `fn parse_book_ticker`, `RECONNECT_BASE`, `RECONNECT_MAX` triggered dead_code errors since Task 2 hadn't added callers yet. Clippy -D warnings would have failed Task 1 commit.
- **Fix:** Added `pub(crate)` visibility. The `#[allow(dead_code)]` attributes added in Task 1 stub were removed in Task 2 once actual callers existed — the final Task 2 implementation has no allow(dead_code) annotations; all items are referenced within the same file.
- **Files modified:** src/data/cex_ws.rs
- **Commit:** 9fbcc07

## Known Stubs

None. The module is complete and self-contained. No placeholder values, no TODO comments, no hardcoded empty collections flowing to UI.

## Threat Flags

No new threat surface beyond what was documented in the plan's threat_model. All six STRIDE threats (T-11-01 through T-11-06) are mitigated or accepted as specified.

## Self-Check: PASSED

- `src/data/cex_ws.rs` exists: FOUND
- `src/data/mod.rs` contains `pub mod cex_ws`: FOUND
- Task 1 commit 9fbcc07: FOUND
- Task 2 commit 95ec104: FOUND
- `cargo test --lib data::cex_ws`: 10 passed
- `cargo build`: success (no new errors; pre-existing fees.rs operator-precedence warning is out of scope)
- No `unwrap()` outside #[cfg(test)] blocks: CONFIRMED (grep returns 0 matches)
- `to_lowercase()` applied before URL construction: CONFIRMED
- No `PING_INTERVAL`: CONFIRMED
- No `accountSubscribe`: CONFIRMED
