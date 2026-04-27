---
phase: 11-cex-price-feed-via-binance-websocket
verified: 2026-04-17T12:00:00Z
status: human_needed
score: 8/9
overrides_applied: 0
human_verification:
  - test: "Run `cargo run -- watch <MINT> --shadow --cex-symbol SOLUSDT` and confirm on-screen price matches Binance within ~5s of startup; logs show connect/frame messages; no stale warn during steady state"
    expected: "Price on-screen matches Binance bookTicker mid within a few cents; logs: 'cex_ws: Binance feed will start for SOLUSDT', 'cex_ws: connecting to wss://stream.binance.com:9443/ws/solusdt@bookTicker', 'cex_ws: connected, waiting for bookTicker frames'"
    why_human: "Requires live network connection to Binance and a real Solana RPC endpoint; cannot test in static code analysis"
  - test: "Run `cargo run -- watch <MINT> --shadow` without --cex-symbol and confirm behavior is unchanged from pre-Phase-11"
    expected: "Log: '--cex-symbol not set, using on-chain price'; on-screen price matches on-chain derivation; no cex_ws log lines"
    why_human: "Requires running the binary against a live RPC endpoint"
  - test: "While watching with --cex-symbol SOLUSDT, disconnect network for 35+ seconds then reconnect"
    expected: "Exactly ONE warn 'cex_ws: price stale >30s, falling back to on-chain sqrt_price'; reconnect attempts logged; exactly ONE info 'cex_ws: price fresh again, resuming CEX feed' on recovery"
    why_human: "Requires inducing a real network outage and observing log transition counts — cannot verify AtomicBool transition semantics without runtime"
  - test: "Press Ctrl+C during a watch run with --cex-symbol"
    expected: "Both 'cex_ws: clean shutdown' (or 'cex_ws: shutdown received, exiting') and WS watch shutdown appear within ~1s; process exits cleanly"
    why_human: "Requires interactive terminal session"
  - test: "Query pnl_history table after ~60s run with --cex-symbol SOLUSDT"
    expected: "price column values match Binance mid-price (not on-chain derivation)"
    why_human: "Requires running DB and a funded (or shadow) watch session"
---

# Phase 11: CEX price feed via Binance WebSocket — Verification Report

**Phase Goal:** Add an independent Binance bookTicker WebSocket price feed (`src/data/cex_ws.rs`) and wire it into the `watch` subcommand so that rebalance decisions, IL, and P&L use a CEX mid-price instead of on-chain sqrt_price.
**Verified:** 2026-04-17T12:00:00Z
**Status:** human_needed
**Re-verification:** No — initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | Binance bookTicker WS session can connect, parse, and drop a frame into shared state without the watch loop running | VERIFIED | `watch_binance_price` / `run_binance_session` / `handle_frame` fully implemented in cex_ws.rs; 10 unit tests pass including `handle_frame_updates_state` |
| 2 | `parse_book_ticker` returns Some(mid) for valid JSON and None for malformed input (no panic) | VERIFIED | 4 parser unit tests pass: valid, malformed_json, missing_bid_ask, non_numeric — all return correct result without panic |
| 3 | Reconnect backoff grows after connect failures and resets after a successful connection | VERIFIED | `backoff_grows_then_resets` and `backoff_saturates_at_max` tests pass; backoff logic matches ws.rs pattern |
| 4 | `CexPriceState` is published as a public type so main.rs can wire it | VERIFIED | `pub type CexPriceState = Arc<RwLock<Option<CexPrice>>>` at line 16 of cex_ws.rs; used in main.rs line 752 |
| 5 | Running `watch` with `--cex-symbol SOLUSDT` spawns Binance task and switches price to Binance mid-price | VERIFIED (code) / NEEDS HUMAN (live) | `data::cex_ws::watch_binance_price` spawned at main.rs line 760 when cex_symbol is Some; `price_current` resolver at lines 815-838 reads from CexPriceState |
| 6 | Running `watch` without `--cex-symbol` preserves on-chain behavior with correct log | VERIFIED (code) / NEEDS HUMAN (live) | None arm falls through to `onchain_price` at main.rs line 836; startup log `--cex-symbol not set, using on-chain price` at line 487 |
| 7 | Stale fallback emits exactly one warn per stale-to-fresh and fresh-to-stale transition | VERIFIED (code) / NEEDS HUMAN (live) | `cex_was_stale: AtomicBool` at main.rs lines 772-773; swap-before-warn pattern at lines 822 and 828 ensures single transition log |
| 8 | `compute_il` and `PnlSnapshot.price` receive CEX-or-fallback price, not raw on-chain | VERIFIED | `analytics::pnl::compute_il(entry_price, price_current, ...)` at line 910; `price: price_current` in PnlSnapshot at line 959 |
| 9 | `should_rebalance` still receives `tick_current_index` unchanged | VERIFIED | `strategy::should_rebalance(pool.tick_current_index, ...)` at line 1080-1081; not modified |

**Score:** 8/9 truths verified (9th requires live human verification)

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `src/data/cex_ws.rs` | watch_binance_price, CexPrice, CexPriceState, parse_book_ticker; min 120 lines | VERIFIED | 233 lines; all four public items present; `pub async fn watch_binance_price` at line 53 |
| `src/data/mod.rs` | `pub mod cex_ws` export | VERIFIED | Line 1: `pub mod cex_ws;` |
| `src/main.rs` | `cex_symbol: Option<String>` CLI field, Binance spawn, CEX-or-fallback resolver, `cex_ws: price stale` log | VERIFIED | All four items confirmed present |

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|-----|--------|---------|
| `src/data/cex_ws.rs` | `wss://stream.binance.com:9443/ws/{symbol}@bookTicker` | `tokio_tungstenite::connect_async` | VERIFIED | `stream.binance.com:9443` in `build_stream_url`; `connect_async(url)` in `run_binance_session` line 96 |
| `src/data/cex_ws.rs` | `Arc<RwLock<Option<CexPrice>>>` | `state.write()` after each parsed frame | VERIFIED | `state.write().unwrap_or_else(|p| p.into_inner())` in `handle_frame` line 35 |
| `src/main.rs Watch arm` | `data::cex_ws::watch_binance_price` | `tokio::spawn` when cex_symbol is Some | VERIFIED | Lines 755-762; `if let Some(ref sym) = cex_symbol` guard |
| `src/main.rs on_notify closure` | `cex_price_state.read()` | `std::sync::RwLock read inside sync Fn closure` | VERIFIED | `cex_price_state_closure.read().unwrap_or_else(...)` at line 817 |
| resolved `price_current` | `analytics::pnl::compute_il(entry_price, price_current, ...)` | function argument | VERIFIED | Line 910-912 |
| resolved `price_current` | `PnlSnapshot { price: price_current, ... }` | struct field | VERIFIED | Line 959 |

### Data-Flow Trace (Level 4)

| Artifact | Data Variable | Source | Produces Real Data | Status |
|----------|---------------|--------|--------------------|--------|
| `src/main.rs on_notify` | `price_current` | `cex_price_state_closure.read()` → `CexPrice.price` set by `handle_frame` → `parse_book_ticker(text)` from live WS frame | Yes — real Binance JSON parsed; fallback via `analytics::greeks::sqrt_q64_to_price(pool.sqrt_price)` | FLOWING (code) / NEEDS HUMAN (live confirmation) |

### Behavioral Spot-Checks

| Behavior | Command | Result | Status |
|----------|---------|--------|--------|
| 10 cex_ws unit tests pass | `cargo test --lib data::cex_ws` | 10 passed, 0 failed | PASS |
| cargo build succeeds | `cargo build` | 0 errors, 1 pre-existing warning (solana-client future-compat) | PASS |
| `--cex-symbol` appears in watch --help | `cargo run -- watch --help` | reported CONFIRMED in 11-02-SUMMARY.md | PASS |
| parse_book_ticker_valid returns correct mid | unit test | 140.25 from (140.20+140.30)/2 | PASS |
| No production unwrap() in cex_ws.rs | grep | 0 matches outside #[cfg(test)] | PASS |
| No PING_INTERVAL or accountSubscribe | grep | 0 matches | PASS |
| Old `* 10f64.powi(9 - 6)` price derivation removed from watch resolver | grep | 0 matches (replaced by DECIMAL_SCALE + resolver block) | PASS |
| Live Binance feed drives on-screen price within 5s | N/A — requires live network | Not testable statically | SKIP |

### Requirements Coverage

Note: CEX-01 through CEX-07 are defined only in the phase plan frontmatter — they do not appear in `.planning/REQUIREMENTS.md` (which covers only the v1.1 research milestone IDs: CENSUS/FILTER/DEEP/LAND/SIZE/SPEC). This is not a gap — Phase 11 is an infrastructure phase for v1.1 and its requirements were defined inline in the plans.

| Requirement | Source Plan | Description (inferred from plan tasks) | Status |
|-------------|------------|----------------------------------------|--------|
| CEX-01 | 11-01-PLAN | `src/data/cex_ws.rs` module with CexPrice, CexPriceState, watch_binance_price | SATISFIED |
| CEX-02 | 11-01-PLAN | `parse_book_ticker` returns mid-price or None (no panic on malformed) | SATISFIED |
| CEX-03 | 11-01-PLAN | Reconnect loop with exponential backoff (1s→30s), reset on successful connect | SATISFIED |
| CEX-04 | 11-02-PLAN | `--cex-symbol` CLI flag on watch subcommand | SATISFIED |
| CEX-05 | 11-01-PLAN | `src/data/mod.rs` exports `pub mod cex_ws` | SATISFIED |
| CEX-06 | 11-02-PLAN | Binance task spawned when --cex-symbol set; CEX price resolver in on_notify | SATISFIED |
| CEX-07 | 11-02-PLAN | Stale fallback (>30s) with transition-only warn; DB price uses CEX-or-fallback | SATISFIED (code verified; live behavior human_needed) |

### Anti-Patterns Found

| File | Line | Pattern | Severity | Impact |
|------|------|---------|----------|--------|
| `src/main.rs` | ~988 | `db_pool.as_ref().unwrap()` inside risk-gate block | Warning | Violates CLAUDE.md no-unwrap rule; panics if future refactor breaks coupling invariant. Pre-existing pattern, not introduced by Phase 11 but flagged in 11-REVIEW.md as WR-01. |
| `src/main.rs` | 485-488 | No validation that `--cex-symbol` is non-empty | Warning | Empty string causes permanent reconnect loop. Flagged in 11-REVIEW.md as WR-02. |
| `src/main.rs` | 759-762 | `JoinHandle` from CEX task spawn silently dropped | Warning | If task panics in future, watch loop silently falls back without operator alert. Flagged in 11-REVIEW.md as WR-03. |
| `src/main.rs` | 563 vs 811 | `10f64.powi(9 - 6)` duplicated alongside `const DECIMAL_SCALE` | Info | Maintenance risk if pool decimals change. Flagged in 11-REVIEW.md as IN-01. |

None of the anti-patterns above block the phase goal. WR-02 (empty symbol) and WR-03 (dropped handle) are robustness concerns, not correctness failures for the normal usage path. The pre-existing `unwrap()` (WR-01) was not introduced by Phase 11.

### Human Verification Required

#### 1. Live Binance feed drives on-screen price

**Test:** `cargo run -- watch <MINT> --shadow --cex-symbol SOLUSDT` — confirm on-screen `Price: $X.XXXX` matches Binance bookTicker mid within ~5s; logs show connection established.
**Expected:** Price within a few cents of https://www.binance.com/en/trade/SOL_USDT mid; logs contain `cex_ws: Binance feed will start for SOLUSDT`, `cex_ws: connecting to wss://stream.binance.com:9443/ws/solusdt@bookTicker`, `cex_ws: connected, waiting for bookTicker frames`; no `cex_ws: price stale` during steady state.
**Why human:** Requires live network connection to Binance and a functional Solana RPC endpoint.

#### 2. No-flag behavior preserved

**Test:** `cargo run -- watch <MINT> --shadow` without `--cex-symbol`.
**Expected:** Log `--cex-symbol not set, using on-chain price`; price matches on-chain derivation; no cex_ws connection log lines.
**Why human:** Requires a running RPC endpoint to confirm on-chain price is used.

#### 3. Stale fallback and recovery — single warn per transition

**Test:** Start with `--cex-symbol SOLUSDT`, wait for first frame, disconnect network for 35+ seconds, reconnect.
**Expected:** Exactly ONE `warn!` `cex_ws: price stale >30s, falling back to on-chain sqrt_price` on disconnect; exactly ONE `info!` `cex_ws: price fresh again, resuming CEX feed` on reconnect.
**Why human:** AtomicBool transition guard is correct in code, but single-warn semantics require runtime observation to confirm count.

#### 4. Ctrl+C shuts down both tasks cleanly

**Test:** Press Ctrl+C during a watch run with `--cex-symbol`.
**Expected:** Both `cex_ws: clean shutdown` (or `cex_ws: shutdown received, exiting`) and WS watch shutdown appear within ~1s; process exits with code 0.
**Why human:** Requires interactive terminal.

#### 5. DB pnl_history.price reflects Binance mid-price

**Test:** Run for ~60s with `--cex-symbol SOLUSDT` and a configured DATABASE_URL; query `SELECT price FROM pnl_history ORDER BY observed_at DESC LIMIT 5`.
**Expected:** `price` column values match Binance mid-price (not raw on-chain value).
**Why human:** Requires a running PostgreSQL/TimescaleDB instance.

### Gaps Summary

No automated gaps were found. All code-verifiable must-haves pass. The `human_needed` status reflects the blocking human-verify checkpoint from Plan 02 Task 3 (the checkpoint task was not executed per the SUMMARY: "Status: AWAITING CHECKPOINT — Tasks 1 and 2 complete; Checkpoint (human-verify) pending").

The three review warnings (WR-01 empty symbol, WR-02 dropped handle, WR-03 pre-existing unwrap) are robustness concerns documented in 11-REVIEW.md. They do not block the stated phase goal but should be addressed before live production use.

---

_Verified: 2026-04-17T12:00:00Z_
_Verifier: Claude (gsd-verifier)_
