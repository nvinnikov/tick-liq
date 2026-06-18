# Phase 11: CEX Price Feed via Binance WebSocket - Research

**Researched:** 2026-04-17
**Domain:** Rust async WebSocket + shared state + Binance stream protocol
**Confidence:** HIGH

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

- **D-01:** Use raw `tokio-tungstenite` — already a dependency, pattern exists in `src/data/ws.rs`. No new crates added.
- **D-02:** Connect to `wss://stream.binance.com:9443/ws/{symbol}@bookTicker`. Parse `b` (best bid) and `a` (best ask) fields; compute mid-price as `(bid + ask) / 2.0`.
- **D-03:** CEX price stored as `Arc<RwLock<Option<f64>>>`. `None` = not yet received first tick. Shared between the Binance WS task and the watch loop.
- **D-04:** Staleness threshold = 30 seconds. If `last_updated` timestamp on the price is older than 30s, fall back to on-chain `sqrt_price` (via `sqrt_q64_to_price`). Log a `warn!` when fallback activates and when CEX price resumes.
- **D-05:** Binance WS task auto-reconnects with exponential backoff — same pattern as `watch_account` in `ws.rs`. No explicit Telegram alert for connectivity issues (on-chain fallback is sufficient safety net for Phase 11).
- **D-06:** Symbol passed via CLI flag `--cex-symbol <SYMBOL>` (e.g. `--cex-symbol SOLUSDT`) on the `watch` subcommand. No default — flag is required when CEX feed is active.
- **D-07:** CEX mid-price replaces on-chain price in all three of:
  1. Rebalance signal evaluation (`tick_current` comparison in `strategy/signal.rs`)
  2. IL calculation (`compute_il()` in `src/math/il.rs`)
  3. P&L DB write (`price` column in `pnl_history` via `src/storage/positions.rs`)
  - On-chain `sqrt_price` is still used for position amount calculations.

### Claude's Discretion

- Internal struct name for the shared price handle (`CexPriceFeed`, `BinancePriceState`, etc.)
- Whether to put the Binance WS logic in a new `src/data/cex_ws.rs` or extend `ws.rs`
- Exact log message wording for fallback activation/recovery

### Deferred Ideas (OUT OF SCOPE)

- Multi-exchange price fallback (Binance → Kraken/Bybit)
- Telegram alert on CEX disconnect
</user_constraints>

---

## Summary

Phase 11 adds a Binance `@bookTicker` WebSocket feed as an independent price source for rebalance decisions, IL calculation, and P&L DB writes — removing circular dependency on the pool's own `sqrt_price`. The implementation closely mirrors the existing `watch_account` pattern in `src/data/ws.rs` with three key differences: (1) Binance uses server-initiated pings every 20s rather than client-initiated pings; (2) connections are forcibly closed by Binance after 24 hours (must be treated as a normal reconnect event); (3) the stream URL requires a **lowercase** symbol even when the CLI flag accepts uppercase.

The shared price state sits in `Arc<RwLock<Option<CexPrice>>>` where `CexPrice` holds both the `f64` mid-price and a `std::time::Instant` for staleness checking. The watch loop resolves the effective price on every tick: CEX mid-price if fresh, on-chain `sqrt_price` fallback if stale or not yet received. `should_rebalance` receives `tick_current: i32` from the on-chain pool for range-boundary checks, while the `price_current: f64` fed to `compute_il` and the DB `price` column is the resolved CEX-or-fallback value.

**Primary recommendation:** Create `src/data/cex_ws.rs` (separate from `ws.rs` — different protocol handshake pattern), expose `watch_binance_price(symbol, price_state, shutdown)`, and keep all staleness resolution logic in `main.rs` at the point of use so the module stays stateless.

---

## Architectural Responsibility Map

| Capability | Primary Tier | Secondary Tier | Rationale |
|------------|-------------|----------------|-----------|
| Binance WS connection & reconnect | Data Layer (`src/data/`) | — | Pure I/O task, no business logic |
| Shared price state (Arc<RwLock>) | Data Layer / main.rs | — | Owned by main, passed to data module |
| Staleness resolution (CEX vs fallback) | main.rs watch loop | — | Decision involves both price sources |
| Rebalance signal evaluation | Strategy Layer (`src/strategy/signal.rs`) | — | Pure function, receives resolved price |
| IL calculation | Math Layer (`src/math/il.rs`) | — | Pure function, receives resolved price |
| P&L DB write | Storage Layer (`src/storage/`) | — | Receives resolved price as parameter |

---

## Standard Stack

### Core (already in Cargo.toml — no new deps)

| Library | Version (locked) | Purpose | Why Standard |
|---------|-----------------|---------|--------------|
| `tokio-tungstenite` | 0.21.0 [VERIFIED: Cargo.lock] | WebSocket client | Already used in `src/data/ws.rs` |
| `futures-util` | 0.3 [VERIFIED: Cargo.toml] | `StreamExt`, `SinkExt` | Same pattern as `ws.rs` |
| `tokio` | 1 (full) [VERIFIED: Cargo.toml] | Async runtime, `RwLock`, `broadcast` | Project-wide |
| `serde_json` | 1 [VERIFIED: Cargo.toml] | Parse bookTicker JSON | Project-wide |
| `tracing` | 0.1 [VERIFIED: Cargo.toml] | `info!`, `warn!` logs | Project-wide |
| `anyhow` | 1 [VERIFIED: Cargo.toml] | Error handling | Required by CLAUDE.md |

**No new dependencies required.** [VERIFIED: all packages present in Cargo.toml]

---

## Architecture Patterns

### System Architecture Diagram

```
CLI --cex-symbol SOLUSDT
          |
          v
   main.rs (Watch cmd)
          |
          +-- tokio::spawn --> watch_binance_price()   <-- src/data/cex_ws.rs
          |                         |
          |                    [reconnect loop]
          |                         |
          |              Binance wss://stream.binance.com:9443
          |              /ws/solusdt@bookTicker
          |                         |
          |              parse {b, a} -> mid = (b+a)/2
          |                         |
          |              Arc<RwLock<Option<CexPrice>>>
          |                     /        \
          |              .price          .updated_at (Instant)
          |
          +-- watch_account() --> Solana WS (existing, unchanged)
                    |
                    v
             on_notify callback (per tick)
                    |
             resolve_price(cex_state, pool.sqrt_price):
               - if cex.updated_at elapsed < 30s  -> CEX mid-price
               - else (stale / None)              -> sqrt_q64_to_price(pool.sqrt_price) * 1000
                    |
             +------+----------+
             |                 |
     compute_il(...)     pnl_history DB write
     strategy::should_rebalance(tick_current_index, ...)
```

### Recommended Project Structure

```
src/data/
├── mod.rs           # add: pub mod cex_ws;
├── ws.rs            # unchanged (Solana WS)
└── cex_ws.rs        # NEW: Binance bookTicker loop
```

The new file is self-contained and parallels `ws.rs`. `mod.rs` gains one line.

### Pattern 1: Binance bookTicker Session Loop

The key structural difference from `ws.rs`: Binance sends server-initiated pings; we do NOT send client-initiated pings. The `ping_interval` and `pong_deadline` from `ws.rs` are removed. Instead we respond to `Message::Ping` from the server immediately with `Message::Pong`.

Also: Binance forcibly closes connections after 24 hours. A `Message::Close` frame must be treated as `SessionResult::Reconnect { connected: true }` — identical to how `ws.rs` handles it.

```rust
// Source: derived from src/data/ws.rs structure + Binance WS docs
// [CITED: https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams]

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};
use tracing::{info, warn};

pub struct CexPrice {
    pub price: f64,
    pub updated_at: Instant,
}

pub type CexPriceState = Arc<RwLock<Option<CexPrice>>>;

/// symbol: uppercase accepted, lowercased internally for the stream URL.
pub async fn watch_binance_price(
    symbol: String,
    state: CexPriceState,
    mut shutdown: broadcast::Receiver<()>,
) {
    let symbol_lower = symbol.to_lowercase();
    let url = format!(
        "wss://stream.binance.com:9443/ws/{}@bookTicker",
        symbol_lower
    );
    let mut backoff = std::time::Duration::from_secs(1);
    const RECONNECT_MAX: std::time::Duration = std::time::Duration::from_secs(30);

    loop {
        if shutdown.try_recv().is_ok() {
            return;
        }
        match run_binance_session(&url, &state, &mut shutdown).await {
            SessionResult::Shutdown => return,
            SessionResult::Reconnect { connected } => {
                if connected { backoff = std::time::Duration::from_secs(1); }
                warn!("cex_ws: disconnected, reconnecting in {:?}", backoff);
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown.recv() => return,
                }
                backoff = (backoff * 2).min(RECONNECT_MAX);
            }
        }
    }
}
```

### Pattern 2: bookTicker Message Parsing

```rust
// Source: Binance WS stream spec
// [CITED: https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams#individual-symbol-book-ticker-streams]
// Message format: {"u":..., "s":"SOLUSDT", "b":"140.25", "B":"10.5", "a":"140.26", "A":"8.3"}

fn parse_book_ticker(text: &str) -> Option<f64> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let bid: f64 = v["b"].as_str()?.parse().ok()?;
    let ask: f64 = v["a"].as_str()?.parse().ok()?;
    Some((bid + ask) / 2.0)
}
```

### Pattern 3: Price Resolution in Watch Loop

```rust
// In main.rs on_notify callback, after fetching pool data:
// [ASSUMED] — pattern shape; exact field names depend on struct name chosen under discretion

const STALE_SECS: u64 = 30;

let price_current: f64 = {
    let guard = cex_price_state.read().await; // or block_in_place for sync context
    match guard.as_ref() {
        Some(cp) if cp.updated_at.elapsed().as_secs() < STALE_SECS => cp.price,
        Some(_) => {
            warn!("cex_ws: price stale >30s, falling back to on-chain sqrt_price");
            analytics::greeks::sqrt_q64_to_price(pool.sqrt_price) * 1e3
        }
        None => {
            // Not yet received — fall back silently (no warn on startup)
            analytics::greeks::sqrt_q64_to_price(pool.sqrt_price) * 1e3
        }
    }
};
```

Note: `block_in_place` is needed because `on_notify` is a sync `Fn` closure (matches existing pattern in `ws.rs`). Use `tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(state.read()))`.

### Pattern 4: Spawning the Binance Task

```rust
// In main.rs Commands::Watch arm, after shutdown channel setup:
// Spawn alongside existing watch_account task.

let cex_price_state: CexPriceState = Arc::new(RwLock::new(None));
let cex_state_clone = Arc::clone(&cex_price_state);
let cex_shutdown_rx = shutdown_tx.subscribe(); // subscribe before watch_account

if let Some(ref sym) = cex_symbol {
    let sym = sym.clone();
    tokio::spawn(async move {
        data::cex_ws::watch_binance_price(sym, cex_state_clone, cex_shutdown_rx).await;
    });
}

// Existing:
data::ws::watch_account(ws_url, pool_addr, shutdown_rx, on_notify).await;
```

### Anti-Patterns to Avoid

- **Client-initiated pings to Binance:** Binance sends server pings; we respond to them. Do NOT copy the `ping_interval` from `ws.rs` to the Binance session — it is unnecessary and counts against the 5 msg/s rate limit [CITED: Binance WS docs].
- **Uppercase stream URL:** `wss://.../ws/SOLUSDT@bookTicker` returns 400. Must lowercase the symbol before URL construction [CITED: Binance WS docs — "Symbols must be lowercase"].
- **Holding the RwLock across tick processing:** Acquire, read, drop. Never hold the lock while calling `compute_il` or DB writes.
- **Using `unwrap()` on price parse:** `b` and `a` are strings from Binance — use `parse::<f64>().ok()?` and log+skip on failure (per `anyhow`/no-unwrap rule in CLAUDE.md).
- **Missing 24h reconnect handling:** Binance closes with a `Close` frame after 24h. This is a normal `SessionResult::Reconnect` — same as `ws.rs`. If not handled, the task silently exits and the price goes stale.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Exponential backoff | Custom retry timer | Same constants/pattern as `ws.rs` | Already tested, proven in production watch loop |
| Shutdown coordination | New channel type | Existing `broadcast::Receiver<()>` | Already threaded through watch loop; subscribe a second receiver |
| Async read from sync closure | Spawn new task | `tokio::task::block_in_place` + `block_on` | Same pattern already used for DB writes in watch loop (line ~877) |

---

## Common Pitfalls

### Pitfall 1: Uppercase Symbol in Stream URL

**What goes wrong:** Connect to `wss://stream.binance.com:9443/ws/SOLUSDT@bookTicker` — Binance returns HTTP 400 and the session immediately fails, triggering backoff loop. CEX price is permanently `None`.

**Why it happens:** Binance stream names are case-sensitive and require lowercase.

**How to avoid:** `symbol.to_lowercase()` before URL construction. Accept uppercase in CLI for UX consistency with Binance trading pair names, lowercase internally.

**Warning signs:** Session always shows `connected: false` in logs despite valid symbol.

### Pitfall 2: Treating 24h Disconnect as a Fatal Error

**What goes wrong:** After 24 hours the Binance server sends a `Close` frame. If the task exits without reconnecting, `CexPrice.updated_at` ages past 30s and the fallback silently activates — but the fallback warn fires every tick rather than once.

**Why it happens:** `Message::Close` from the server ends the stream. If the outer reconnect loop isn't running, the task exits.

**How to avoid:** The outer `watch_binance_price` loop handles this identically to `ws.rs` — `Message::Close` returns `Reconnect { connected: true }`, which resets backoff and loops.

**Warning signs:** `warn!("cex_ws: price stale")` firing continuously around the 24h mark.

### Pitfall 3: Staleness Warn Fires on Every Tick During Startup

**What goes wrong:** Between watch start and receipt of first bookTicker message, every tick logs a warn about stale price, creating noisy logs.

**Why it happens:** The staleness check doesn't distinguish `None` (never received) from `Some(stale)`.

**How to avoid:** Separate the two cases explicitly: `None` = silent fallback (startup grace), `Some(cp)` where elapsed > 30s = `warn!` (genuine disconnection). See Pattern 3 above.

**Warning signs:** Warn spam in the first few seconds of watch startup.

### Pitfall 4: RwLock Deadlock in Sync Closure

**What goes wrong:** `on_notify` is a sync `Fn` closure. Calling `.await` inside it panics. Using `std::sync::RwLock` instead of `tokio::sync::RwLock` avoids the await but still blocks the executor thread if used incorrectly.

**Why it happens:** Mixed sync/async boundary.

**How to avoid:** Use `std::sync::RwLock` (not tokio's) for the `CexPriceState` — this allows `state.read().unwrap()` in sync context without needing `block_in_place`. The write side (Binance task) is fully async and can use std RwLock's `.write().unwrap()` since it's in its own `tokio::spawn`.

**Alternatively:** Keep tokio RwLock and wrap reads in `block_in_place` (same as DB writes at line 877) — both approaches work; std RwLock is simpler for this use case.

### Pitfall 5: `should_rebalance` Still Uses `tick_current_index` (Not Price)

**What goes wrong:** Confusion over D-07: the signal function compares *ticks*, not prices. The CEX price does NOT replace `tick_current_index` in `should_rebalance`. It replaces `price_current` in `compute_il` and the DB write. `tick_current_index` stays on-chain.

**Why it happens:** D-07 says "rebalance signal evaluation" — but signal.rs evaluates tick proximity, not price proximity. The CEX price is only used for the `net_pnl_usd` argument (computed from IL which uses CEX price) and the `price` column.

**How to avoid:** Keep `pool.tick_current_index` as the first argument to `strategy::should_rebalance`. Only pipe the resolved `price_current` into `compute_il` and `PnlSnapshot.price`.

---

## Code Examples

### Verified: bookTicker JSON message format

```json
// Source: Binance API docs
// [CITED: https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams#individual-symbol-book-ticker-streams]
{
  "u": 400900217,
  "s": "BNBUSDT",
  "b": "25.35190000",
  "B": "31.21000000",
  "a": "25.36520000",
  "A": "40.66000000"
}
```

### Verified: existing ws.rs reconnect constants to replicate

```rust
// Source: src/data/ws.rs (verified by reading)
// [VERIFIED: codebase]
const RECONNECT_BASE: Duration = Duration::from_secs(1);
const RECONNECT_MAX: Duration = Duration::from_secs(30);
```

### Verified: block_in_place pattern already used in watch loop

```rust
// Source: src/main.rs line ~877 (verified by reading)
// [VERIFIED: codebase]
let write_result = tokio::task::block_in_place(|| {
    tokio::runtime::Handle::current()
        .block_on(storage::writer::write_pool_tick(pg, &tick))
});
```

### Verified: existing fallback price derivation (what we fall back TO)

```rust
// Source: src/main.rs line ~770 (verified by reading)
// [VERIFIED: codebase]
let price_current = analytics::greeks::sqrt_q64_to_price(pool.sqrt_price) * 10f64.powi(9 - 6);
// 10^(9-6) = 1000 — SOL(9 decimals) / USDC(6 decimals) scaling
```

---

## Runtime State Inventory

> Not applicable — this is a greenfield feature addition, not a rename/refactor/migration phase.

---

## Environment Availability

| Dependency | Required By | Available | Version | Fallback |
|------------|------------|-----------|---------|----------|
| `tokio-tungstenite` | Binance WS connection | Yes | 0.21.0 [VERIFIED: Cargo.lock] | — |
| Binance WS endpoint | CEX price feed | Assumed reachable [ASSUMED] | — | On-chain sqrt_price (D-04) |
| PostgreSQL / TimescaleDB | P&L DB write with CEX price | Not checked in this session [ASSUMED] | — | Watch runs without DB (existing behavior) |

**Missing dependencies with no fallback:** None.

**Note on Binance endpoint reachability:** The endpoint `wss://stream.binance.com:9443` is a public, unauthenticated stream. No API key is required. Reachability depends on network/firewall; the reconnect loop handles transient outages. [CITED: Binance docs — public stream, no auth required]

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `tick_current_index` → price via sqrt_q64 for signal/IL/P&L | Binance mid-price as independent reference | Phase 11 | Eliminates circular: pool-being-traded-on sets the price used to decide whether to trade |
| Client-initiated WS ping (Solana) | Server-initiated ping (Binance) | N/A — different protocols | Must respond to Binance pings, not send own |

---

## Open Questions

1. **`std::sync::RwLock` vs `tokio::sync::RwLock` for CexPriceState**
   - What we know: `on_notify` is a sync `Fn` closure; tokio RwLock requires `.await`; std RwLock does not.
   - What's unclear: Whether using std RwLock write-lock from async Binance task is acceptable (it is, as long as the lock is never held across `.await` points).
   - Recommendation: Use `std::sync::RwLock` — simpler, no block_in_place needed on the read side.

2. **Optional vs required `--cex-symbol` flag**
   - What we know: D-06 says "no default — flag is required when CEX feed is active." This implies the flag is optional on the CLI (watch still works without it) but there is no default value.
   - What's unclear: Should CEX feed be silently skipped when `--cex-symbol` is absent, or should the user always pass it?
   - Recommendation: Make it `Option<String>` in clap. If absent, skip spawning the Binance task and use on-chain price throughout (existing behavior preserved). Add a startup log if absent: `info!("--cex-symbol not set, using on-chain price")`.

3. **Decimal scaling consistency for fallback price**
   - What we know: The watch loop already applies `* 10^(9-6)` = `* 1000` to `sqrt_q64_to_price` result for SOL/USDC.
   - What's unclear: The CEX price from Binance `SOLUSDT` is natively in USD, so it needs NO decimal scaling — it's already the correct unit. The fallback (on-chain) needs `* 1000`. Must not accidentally apply scaling to the CEX price.
   - Recommendation: Resolve this explicitly in the price resolution helper: CEX path returns `cp.price` as-is; fallback path applies `* scale_factor`.

---

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | Binance WS endpoint is reachable from the deployment network | Environment Availability | Startup fallback to on-chain price; reconnect loop handles it |
| A2 | SOLUSDT bookTicker is available on Binance spot (not just perpetuals) | Standard Stack | Need to verify symbol exists; `stream.binance.com` is spot |
| A3 | PostgreSQL is available when CEX price is active | Environment Availability | No DB impact — DB write just receives a different float value |
| A4 | `std::sync::RwLock` write from async task is safe (lock not held across await) | Architecture Patterns | Deadlock if held across await — mitigated by design |

---

## Sources

### Primary (HIGH confidence)
- [VERIFIED: Cargo.lock / Cargo.toml] — `tokio-tungstenite 0.21.0`, all project dependencies
- [VERIFIED: src/data/ws.rs] — existing reconnect pattern, ping/pong handling, SessionResult enum
- [VERIFIED: src/main.rs] — watch loop structure, on_notify closure type, block_in_place usage, `should_rebalance` call site
- [VERIFIED: src/math/il.rs] — `compute_il` signature and semantics
- [VERIFIED: src/strategy/signal.rs] — `should_rebalance` signature, tick-based logic
- [CITED: https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams#individual-symbol-book-ticker-streams] — bookTicker message format, field names, update frequency
- [CITED: https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams#websocket-limits] — 5 msg/s limit, server-initiated ping every 20s, 1-min pong timeout
- [CITED: https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams#general-wss-information] — 24-hour connection limit, lowercase symbol requirement

### Secondary (MEDIUM confidence)
- [VERIFIED: cargo search result] — tokio-tungstenite 0.29 is current registry latest; project intentionally stays on 0.21 (D-01)

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — all deps verified in Cargo.toml/Cargo.lock
- Architecture: HIGH — existing ws.rs pattern read directly; Binance protocol verified from official docs
- Pitfalls: HIGH — most derived from verified protocol spec differences and codebase reading

**Research date:** 2026-04-17
**Valid until:** 2026-05-17 (Binance stream API is stable; Rust dep versions pinned)
