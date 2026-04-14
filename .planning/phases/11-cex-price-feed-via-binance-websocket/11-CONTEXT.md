# Phase 11: CEX price feed via Binance WebSocket - Context

**Gathered:** 2026-04-17
**Status:** Ready for planning

<domain>
## Phase Boundary

Add an independent Binance WebSocket price feed (`@bookTicker` stream) to the watch loop so that rebalance decisions, IL calculations, and P&L DB writes use CEX mid-price instead of the on-chain pool price (`sqrt_price` / `tick_current_index`). Only `src/` is touched in this phase.

</domain>

<decisions>
## Implementation Decisions

### Connection & Library
- **D-01:** Use raw `tokio-tungstenite` — already a dependency, pattern exists in `src/data/ws.rs`. No new crates added.
- **D-02:** Connect to `wss://stream.binance.com:9443/ws/{symbol}@bookTicker`. Parse `b` (best bid) and `a` (best ask) fields; compute mid-price as `(bid + ask) / 2.0`.

### Shared State
- **D-03:** CEX price stored as `Arc<RwLock<Option<f64>>>`. `None` = not yet received first tick. Shared between the Binance WS task and the watch loop.

### Fallback on Disconnect
- **D-04:** Staleness threshold = **30 seconds**. If `last_updated` timestamp on the price is older than 30s, fall back to on-chain `sqrt_price` (via `sqrt_q64_to_price`). Log a `warn!` when fallback activates and when CEX price resumes.
- **D-05:** Binance WS task auto-reconnects with exponential backoff — same pattern as `watch_account` in `ws.rs`. No explicit Telegram alert for connectivity issues (on-chain fallback is sufficient safety net for Phase 11).

### Symbol Configuration
- **D-06:** Symbol passed via CLI flag `--cex-symbol <SYMBOL>` (e.g. `--cex-symbol SOLUSDT`) on the `watch` subcommand. No default — flag is required when CEX feed is active.

### Scope of Replacement
- **D-07:** CEX mid-price replaces on-chain price in **all three** of:
  1. Rebalance signal evaluation (`tick_current` comparison against range boundaries in `strategy/signal.rs`)
  2. IL calculation (`calculate_il()` in `src/math/il.rs`)
  3. P&L DB write (`price` column in `pnl_history` via `src/storage/positions.rs`)
- On-chain `sqrt_price` is still used for position amount calculations (liquidity math) — only the "current price for P&L/signal" is swapped.

### Claude's Discretion
- Internal struct name for the shared price handle (`CexPriceFeed`, `BinancePriceState`, etc.)
- Whether to put the Binance WS logic in a new `src/data/cex_ws.rs` or extend `ws.rs`
- Exact log message wording for fallback activation/recovery

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Existing WS pattern (replicate this)
- `src/data/ws.rs` — tokio-tungstenite reconnect loop with ping/pong and broadcast shutdown. New Binance WS task should follow this structure exactly.

### Price derivation (understand what we replace)
- `src/main.rs:266-267` — `sqrt_q64_to_price(pool.sqrt_price)` — on-chain price derivation being replaced for signal/IL/P&L
- `src/strategy/signal.rs:31,37,49,55` — `tick_current: i32` usage in rebalance signal

### IL math (receives CEX price)
- `src/math/il.rs` — `calculate_il(entry_price, current_price, ...)` — `current_price` arg becomes CEX mid-price

### DB writes (price column)
- `src/storage/positions.rs:84,92,98` — `il_usd`, `price` columns in `pnl_history` insert

### Binance WS stream spec
- Endpoint: `wss://stream.binance.com:9443/ws/{symbol}@bookTicker`
- Message fields: `b` = best bid price (string), `a` = best ask price (string)
- Mid-price: `(b.parse::<f64>() + a.parse::<f64>()) / 2.0`
- Update frequency: real-time on any top-of-book change

### Deferred multi-exchange fallback reference
- Kraken WS v2 docs: https://docs.kraken.com/api/docs/guides/spot-ws-intro (for future Phase)

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `src/data/ws.rs` — `watch_account()` function: tokio-tungstenite loop with exponential backoff reconnect, ping/pong keepalive, broadcast shutdown channel. New `watch_cex_price()` should mirror this signature and structure.
- `src/math/greeks.rs` — `sqrt_q64_to_price()` — used as fallback when CEX price is stale.

### Established Patterns
- Shutdown via `broadcast::Receiver<()>` — reuse the existing shutdown channel already threaded through watch loop.
- `tracing::{info, warn}` — use same tracing macros for fallback activation logs.
- Keypair/config via env vars — no change needed here (CEX feed is read-only, no auth).

### Integration Points
- `src/main.rs` watch subcommand — spawn Binance WS task alongside existing `watch_account` task; pass `Arc<RwLock<Option<f64>>>` and last-updated timestamp into both.
- `src/strategy/signal.rs` — `should_rebalance(tick_current, ...)` signature will need `cex_price: Option<f64>` parameter or the caller resolves price before calling.
- `src/math/il.rs` — `calculate_il(entry_price, current_price, ...)` — caller passes resolved price (CEX or fallback on-chain).

</code_context>

<specifics>
## Specific Ideas

- Use `@bookTicker` stream (not `@aggTrade`) — fastest price signal, real-time on any bid/ask change.
- Mid-price formula: `(bid + ask) / 2.0` gives a manipulation-resistant reference price not derived from the pool being market-made.
- Staleness check: store `Instant` alongside price in the RwLock struct: `struct CexPrice { price: f64, updated_at: Instant }` — wrap in `Option<CexPrice>`.
- Deferred: multi-exchange fallback (Binance → Kraken/Bybit) is a good follow-up phase once this is stable.

</specifics>

<deferred>
## Deferred Ideas

- **Multi-exchange price fallback** — if Binance WS fails, fall back to Kraken or Bybit instead of on-chain price. Kraken WS v2: https://docs.kraken.com/api/docs/guides/spot-ws-intro. Deferred to a follow-up phase after Phase 11 is stable.
- **Telegram alert on CEX disconnect** — notify operator when falling back to on-chain price. Deferred; on-chain fallback is sufficient safety net for now.

</deferred>

---

*Phase: 11-cex-price-feed-via-binance-websocket*
*Context gathered: 2026-04-17*
