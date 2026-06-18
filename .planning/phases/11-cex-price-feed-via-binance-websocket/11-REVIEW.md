---
phase: 11-cex-price-feed-via-binance-websocket
reviewed: 2026-04-17T00:00:00Z
depth: standard
files_reviewed: 3
files_reviewed_list:
  - src/data/cex_ws.rs
  - src/data/mod.rs
  - src/main.rs
findings:
  critical: 0
  warning: 3
  info: 2
  total: 5
status: issues_found
---

# Phase 11: Code Review Report

**Reviewed:** 2026-04-17
**Depth:** standard
**Files Reviewed:** 3
**Status:** issues_found

## Summary

Phase 11 introduces the Binance `bookTicker` WebSocket price feed (`src/data/cex_ws.rs`) and wires it into the `Watch` command in `src/main.rs`. The core WS implementation is clean: frame parsing, exponential backoff, Ping/Pong handling, graceful shutdown, and the stale-price fallback are all correctly implemented. Tests are thorough.

Three warnings were found: an implicit `unwrap()` that panics if a code-path invariant breaks, missing validation of the `cex_symbol` input (empty string causes a permanent reconnect loop), and a `JoinHandle` that is silently dropped. Two info items flag a magic-number inconsistency and a semantically inconsistent P&L expression.

---

## Warnings

### WR-01: `db_pool.as_ref().unwrap()` inside risk-gate block — hidden panic

**File:** `src/main.rs:988`

**Issue:** `db_pool.as_ref().unwrap()` is called inside a block entered only when `snap_opt` is `Some`. `snap_opt` is `Some` only when `db_pool` is `Some` (lines 921–967). The invariant is real, but the compiler cannot see it — the `unwrap()` will panic if a future refactor breaks the coupling (e.g., `snap_opt` is produced by a different code path). The project's `CLAUDE.md` explicitly forbids `unwrap()` in production paths.

**Fix:**
```rust
// Replace line 988 with an explicit guard that propagates the error
let pg_for_persist = match db_pool.as_ref() {
    Some(pg) => pg.clone(),
    None => {
        tracing::warn!("risk: db_pool unexpectedly None inside risk gate, skipping persist");
        return;
    }
};
```

---

### WR-02: Empty `--cex-symbol` causes a permanent reconnect loop

**File:** `src/main.rs:485-488` / `src/data/cex_ws.rs:25-30`

**Issue:** No validation is performed on the `cex_symbol` value before it is passed to `watch_binance_price`. An empty string (e.g. `--cex-symbol ""`) builds the URL `wss://stream.binance.com:9443/ws/@bookTicker`. Binance returns a 400 error, the connect fails, and the reconnect loop retries every 1 → 2 → 4 … → 30 seconds indefinitely, silently filling logs. The user has no feedback that the argument is invalid.

**Fix — validate at argument parsing time in `main.rs`:**
```rust
// After the cex_symbol arm in the Watch match block (around line 485):
if let Some(sym) = cex_symbol {
    if sym.trim().is_empty() {
        anyhow::bail!("--cex-symbol must not be empty");
    }
    tracing::info!("cex_ws: Binance feed will start for {}", sym);
} else {
    tracing::info!("--cex-symbol not set, using on-chain price");
}
```

Alternatively add the check inside `watch_binance_price` to keep the contract at the module boundary:
```rust
// src/data/cex_ws.rs — top of watch_binance_price
if symbol.trim().is_empty() {
    tracing::error!("cex_ws: symbol is empty, aborting");
    return;
}
```

---

### WR-03: Spawned CEX feed `JoinHandle` is silently dropped

**File:** `src/main.rs:759-762`

**Issue:** `tokio::spawn(...)` returns a `JoinHandle<()>` which is immediately dropped. If the task exits unexpectedly (e.g. due to a panic added in future work), the watch loop continues without a price feed and silently falls back to on-chain prices indefinitely, with no operator alert.

**Fix:** Retain the handle and log if it exits unexpectedly. The simplest approach is to store the handle and abort on drop — or at minimum warn:
```rust
let _cex_handle = if let Some(ref sym) = cex_symbol {
    let sym_owned = sym.clone();
    let state_clone = std::sync::Arc::clone(&cex_price_state);
    let cex_shutdown = shutdown_tx.subscribe();
    Some(tokio::spawn(async move {
        data::cex_ws::watch_binance_price(sym_owned, state_clone, cex_shutdown).await;
        tracing::warn!("cex_ws: feed task exited");
    }))
} else {
    None
};
```

---

## Info

### IN-01: Magic number `10f64.powi(9 - 6)` vs `DECIMAL_SCALE` — duplicated constant

**File:** `src/main.rs:563` and `src/main.rs:811-814`

**Issue:** The decimal scale factor for SOL/USDC (10^3 = 1000.0) appears twice: once as `10f64.powi(9 - 6)` on line 563 (entry price at watch start) and once as the `const DECIMAL_SCALE: f64 = 1000.0` defined at line 811 inside the closure. If the pool's token decimals change or the wiring is extended to non-SOL/USDC pairs, one site could be updated while the other is missed.

**Fix:** Define `DECIMAL_SCALE` (or a more descriptive name like `SOL_USDC_DECIMAL_SCALE`) once at module level or at the top of the `Watch` arm, before the closure, and reference it in both places.

---

### IN-02: Semantically inconsistent net P&L expressions

**File:** `src/main.rs:957` and `src/main.rs:1084`

**Issue:** Net P&L is computed two different ways in the same tick callback:
- Line 957 (stored to DB): `computed_fees_earned - computed_il_usd.abs()`
- Line 1084 (passed to `should_rebalance`): `computed_fees_earned + computed_il_usd`

Both evaluate to the same numerical result because `il_usd` is non-positive (IL is a loss), so `+il_usd == -|il_usd|`. However the two expressions look contradictory to a reader and create a maintenance risk: if `il_usd` is ever made positive-convention, one site will be correct and the other wrong.

**Fix:** Use a single expression consistently. The additive form `fees + il_usd` (where `il_usd ≤ 0`) is the cleaner convention since it directly reflects the signed P&L identity. Update line 957 accordingly:
```rust
net_pnl: computed_fees_earned + computed_il_usd,
```

---

_Reviewed: 2026-04-17_
_Reviewer: Claude (gsd-code-reviewer)_
_Depth: standard_
