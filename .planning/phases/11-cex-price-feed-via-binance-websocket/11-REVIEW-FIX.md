---
phase: 11-cex-price-feed-via-binance-websocket
fixed_at: 2026-04-17T00:00:00Z
review_path: .planning/phases/11-cex-price-feed-via-binance-websocket/11-REVIEW.md
iteration: 1
findings_in_scope: 3
fixed: 3
skipped: 0
status: all_fixed
---

# Phase 11: Code Review Fix Report

**Fixed at:** 2026-04-17
**Source review:** .planning/phases/11-cex-price-feed-via-binance-websocket/11-REVIEW.md
**Iteration:** 1

**Summary:**
- Findings in scope: 3
- Fixed: 3
- Skipped: 0

## Fixed Issues

### WR-01: `db_pool.as_ref().unwrap()` inside risk-gate block — hidden panic

**Files modified:** `src/main.rs`
**Commit:** a7c8cea
**Applied fix:** Replaced `db_pool.as_ref().unwrap().clone()` at line 988 with a `match` expression that returns early with a `tracing::warn!` if `db_pool` is unexpectedly `None`, eliminating the potential panic.

### WR-02: Empty `--cex-symbol` causes a permanent reconnect loop

**Files modified:** `src/main.rs`
**Commit:** 1cdcaf5
**Applied fix:** Added an `is_empty()` check on the trimmed `cex_symbol` value inside the `Some(sym)` arm of the existing match block (line 485). Returns `anyhow::bail!` with a descriptive error message before any connection attempt is made.

### WR-03: Spawned CEX feed `JoinHandle` is silently dropped

**Files modified:** `src/main.rs`
**Commit:** ca5d955
**Applied fix:** Changed the `if let` spawn block to an `if/else` expression assigned to `let _cex_handle`. The task closure now calls `tracing::warn!("cex_ws: feed task exited")` after `watch_binance_price` returns, so unexpected exits are visible in logs.

---

_Fixed: 2026-04-17_
_Fixer: Claude (gsd-code-fixer)_
_Iteration: 1_
