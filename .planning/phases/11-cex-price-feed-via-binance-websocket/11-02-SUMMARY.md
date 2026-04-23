---
phase: 11-cex-price-feed-via-binance-websocket
plan: "02"
subsystem: cli/watch
tags: [cli, watch-loop, integration, fallback, cex, binance]
requirements: [CEX-04, CEX-06, CEX-07]

dependency_graph:
  requires:
    - src/data/cex_ws.rs (watch_binance_price, CexPrice, CexPriceState — from Plan 01)
  provides:
    - src/main.rs (--cex-symbol flag, Binance task spawn, CEX-or-fallback price resolver)
  affects:
    - src/main.rs Watch subcommand
    - compute_il / PnlSnapshot.price now receive CEX-or-fallback price when feed is fresh

tech_stack:
  added: []
  patterns:
    - std::sync::Arc<AtomicBool> for transition-only stale/fresh warn guard
    - shutdown_tx.subscribe() before Ctrl+C spawn so Binance task receives same signal
    - const CEX_STALE_SECS / DECIMAL_SCALE inside closure for clarity + compiler inlining
    - unwrap_or_else(|poisoned| poisoned.into_inner()) for RwLock poison recovery (T-11-11)

key_files:
  created: []
  modified:
    - src/main.rs

decisions:
  - Binance spawn placed BEFORE Ctrl+C tokio::spawn so shutdown_tx is not yet moved when subscribe() is called
  - cex_was_stale AtomicBool cloned into closure (not a module-level static) for clean per-run teardown
  - CEX_STALE_SECS = 30s matches plan spec; DECIMAL_SCALE = 1000.0 replaces 10f64.powi(9-6) for readability
  - None arm (startup grace) silently falls back to on-chain — no log spam during first seconds

metrics:
  duration_seconds: 480
  completed_date: "2026-04-17"
  tasks_completed: 2
  tasks_total: 3
  files_created: 0
  files_modified: 1
---

# Phase 11 Plan 02: Watch subcommand CEX-or-fallback price wiring Summary

**One-liner:** `--cex-symbol` CLI flag wires Plan 01 Binance feed into the watch loop — CEX mid-price drives IL / P&L / DB writes, with AtomicBool transition-only stale/fresh logging and on-chain fallback when feed is >30s stale.

**Status: AWAITING CHECKPOINT** — Tasks 1 and 2 complete; Checkpoint (human-verify) pending.

## Changed Line Ranges in src/main.rs

| Change | Location (post-edit) | Description |
|--------|----------------------|-------------|
| Watch struct field | lines 97-103 | `cex_symbol: Option<String>` field added after `entry_price` |
| Watch arm destructure | line 483 | `cex_symbol,` appended to pattern |
| Startup log | lines 485-488 | `match &cex_symbol` → feed will start / not set log |
| CexPriceState + Binance spawn | lines 752-769 | Create Arc<RwLock>, spawn Binance task before Ctrl+C move |
| AtomicBool + closure clones | lines 772-775 | `cex_was_stale`, `cex_was_stale_closure`, `cex_price_state_closure` |
| Price resolver | lines 805-838 | Replaces old 2-line derivation; 30s staleness check + transition logging |

## Checkpoint Verification (Pending)

Checkpoint requires live human verification of five scenarios:
1. Binance feed starts and price matches live Binance within ~5s
2. `--cex-symbol` absent → on-chain price used, correct log emitted
3. Network disconnect → single stale warn, reconnect, single fresh info
4. Ctrl+C shuts down both WS tasks cleanly
5. `pnl_history.price` in DB matches Binance mid-price

Log excerpts and screenshots to be added after checkpoint passes.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Binance spawn must precede Ctrl+C spawn**
- **Found during:** Task 2 implementation
- **Issue:** Plan said insert Binance spawn "AFTER" the Ctrl+C task (line 739), but `shutdown_tx` is moved into the Ctrl+C closure. Calling `shutdown_tx.subscribe()` after that move is a compile error.
- **Fix:** Inserted Binance spawn BEFORE the Ctrl+C `tokio::spawn`. This is exactly what the plan's own "Note" paragraph anticipated: "If it has already been moved, the spawn MUST happen BEFORE the move."
- **Files modified:** src/main.rs
- **Commit:** afbf4ad

## Known Stubs

None. All wiring is live — CexPriceState flows from real Binance frames into `price_current` on every `on_notify` tick. No placeholder values, no TODO comments.

## Threat Flags

No new threat surface beyond plan's threat_model (T-11-10 through T-11-16). All mitigations applied:
- T-11-11 (RwLock poisoning): `unwrap_or_else(|poisoned| poisoned.into_inner())` on reader side
- T-11-12 (log flood): `cex_was_stale` AtomicBool ensures single warn per stale transition
- T-11-15 (CEX bypassing range checks): `should_rebalance(pool.tick_current_index, ...)` unchanged

## Self-Check: PASSED

- `src/main.rs` modified: CONFIRMED
- Task 1 commit 66b6dd5: FOUND
- Task 2 commit afbf4ad: FOUND
- `cargo build`: success (0 errors)
- `cargo test --lib`: 154 passed, 0 failed
- `cargo clippy --lib -- -D warnings`: 0 errors, 0 warnings
- `cargo run -- watch --help` contains `--cex-symbol`: CONFIRMED
- `grep "cex_symbol: Option<String>"` returns 1 match: CONFIRMED
- `grep "data::cex_ws::watch_binance_price"` returns 1 match (spawn): CONFIRMED
- `grep "sqrt_q64_to_price(pool.sqrt_price) * 10f64.powi(9 - 6)"` returns 0 matches: CONFIRMED
- `should_rebalance(pool.tick_current_index, ...)` unchanged: CONFIRMED
- No new raw `.unwrap()` calls added: CONFIRMED
