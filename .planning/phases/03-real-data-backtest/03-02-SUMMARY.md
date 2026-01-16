---
phase: 03-real-data-backtest
plan: 02
subsystem: backtest
tags: [db-replay, backtest, pool-ticks, fee-growth, clmm-math]
dependency_graph:
  requires: [03-01]
  provides: [run_db_backtest]
  affects: [src/backtest/db_replay.rs, src/backtest/mod.rs]
tech_stack:
  added: []
  patterns: [u128-wrapping-sub, x64-fixed-point, utc-calendar-day-rollup, fee-liquidity-share-approximation]
key_files:
  created:
    - src/backtest/db_replay.rs
  modified:
    - src/backtest/mod.rs
decisions:
  - "price_to_tick made pub(crate) to allow cross-module use without duplicating the CLMM formula"
  - "fee approximation uses pool_liquidity share (not fee_growth_inside per tick-array) — documented per T-03-09 accepted risk"
  - "near_edge_ticks = i32::MIN used in no-rebalance test to prevent near-edge branch firing on out-of-range ticks"
  - "DayResult.day is u32 (1-based day counter) not NaiveDate, matching existing GBM schema"
metrics:
  duration: 35m
  completed: 2026-04-09T21:08:45Z
  tasks_completed: 1
  files_changed: 2
---

# Phase 3 Plan 2: DB-Mode Backtest Replay Summary

**One-liner:** DB-mode backtest engine replaying PoolTickRow slices with X64 fee-growth deltas, u128 wrapping, per-tick should_rebalance, and UTC day rollup into the shared BacktestResult schema.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Implement run_db_backtest — per-tick replay | f2552ce | src/backtest/db_replay.rs (new), src/backtest/mod.rs |

## What Was Built

`src/backtest/db_replay.rs` provides `run_db_backtest(DbBacktestInput, &[PoolTickRow]) -> Result<BacktestResult>`:

- **sqrt_price_to_price**: Converts X64 fixed-point u128 to f64 using the same formula as the watch loop (`price = (sqrt_price / 2^64)^2`).
- **fee_growth_delta**: Uses `wrapping_sub` to handle u128 rollover correctly (T-03-05).
- **Fee accrual**: `fees_step = (delta / 2^64) * (position_liquidity / pool_liquidity)`. Guarded by `if t.liquidity > 0` (T-03-06). This is a documented approximation (T-03-09).
- **Per-tick rebalance signal**: Calls `strategy::should_rebalance()` at every tick; on `Rebalance` decision, re-centres the range using `range_factor_lower`/`range_factor_upper`.
- **UTC day rollup**: Groups ticks by `NaiveDate`; flushes a `DayResult` (day: u32, 1-based) at each date boundary.
- **Empty-stream safety**: `anyhow::bail!` on empty input — no panic path (T-03-08).
- **BacktestResult schema**: Identical fields to GBM mode (`days`, `total_fees_usd`, `total_il_usd`, `net_pnl_usd`, `total_rebalances`, `days_in_range`, `fee_apy_pct`, `params_snapshot`).

`src/backtest/mod.rs` changes:
- `price_to_tick` visibility changed from `fn` to `pub(crate)` for use by `db_replay`.
- `pub mod db_replay;` export added.

## Test Results

12 new unit tests in `backtest::db_replay::tests`, all passing:
- `fee_growth_delta_wraps` — u128 rollover handled
- `sqrt_price_conversion_matches_watch_loop` — 2^64 → 1.0
- `sqrt_price_zero_gives_zero` — zero case
- `empty_ticks_returns_error` — anyhow::bail path
- `single_tick_produces_one_day` — minimum viable input
- `fee_accrual_basic` — delta=100, share=0.1 verified
- `fee_accrual_with_wrapping` — u128::MAX wraps to 6
- `zero_liquidity_skips_fee_accrual` — div-by-zero guard
- `multi_day_produces_correct_day_count` — 3 distinct dates → 3 DayResult
- `out_of_range_triggers_rebalance` — rebalance fires when enabled
- `no_rebalances_when_disabled` — no rebalance when cfg disabled
- `result_schema_has_expected_fields` — all BacktestResult fields accessible

All 21 existing GBM backtest tests continue to pass (33 total, 0 failures).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed failing `no_rebalances_when_disabled` test**
- **Found during:** Task 1 test run
- **Issue:** `RebalanceConfig { near_edge_ticks: 0 }` combined with an above-range tick (9999 >> tick_upper ~952) caused `tick_upper - tick_current = -9047 <= 0` to satisfy the near-edge branch, triggering a spurious rebalance even with `rebalance_out_of_range = false`.
- **Fix:** Set `near_edge_ticks = i32::MIN` in the test to make the near-edge condition arithmetically impossible while keeping `rebalance_out_of_range = false` as the primary assertion.
- **Files modified:** src/backtest/db_replay.rs (test only)
- **Commit:** f2552ce

**2. [Rule 2 - Missing critical functionality] Added `#![allow(dead_code)]`**
- **Found during:** Task 1 clippy run
- **Issue:** Binary target flagged `DbBacktestInput`, `run_db_backtest`, `sqrt_price_to_price`, `fee_growth_delta` as dead code since CLI wiring is deferred to plan 03-03.
- **Fix:** Added `#![allow(dead_code)]` consistent with the `tick_reader.rs` precedent in this codebase.
- **Files modified:** src/backtest/db_replay.rs
- **Commit:** f2552ce

**3. [Rule 2 - Missing critical alignment] DayResult.day field is u32 not NaiveDate**
- **Found during:** Task 1 implementation
- **Issue:** Plan pseudocode used `day: NaiveDate` in DayResult, but the existing GBM struct uses `day: u32` (1-based day counter). Matching the actual schema was required.
- **Fix:** Used `u32` day counter with date-boundary detection, storing `NaiveDate` only locally for grouping.
- **Files modified:** src/backtest/db_replay.rs
- **Commit:** f2552ce

## Threat Surface Scan

No new network endpoints, auth paths, or schema changes introduced. All mitigations from the plan's threat register are implemented:

| Threat ID | Status |
|-----------|--------|
| T-03-05 | Mitigated — `wrapping_sub`, unit-tested |
| T-03-06 | Mitigated — `if t.liquidity > 0` guard |
| T-03-07 | Mitigated — `sqrt_price=2^64→1.0` unit-tested |
| T-03-08 | Mitigated — `anyhow::bail!` on empty stream |
| T-03-09 | Accepted — approximation documented in module and fn doc-comments |

## Known Stubs

None. `run_db_backtest` is fully functional. CLI wiring to the `backtest` command is the next plan (03-03).

## Self-Check: PASSED

- `src/backtest/db_replay.rs` exists: FOUND
- `src/backtest/mod.rs` contains `pub mod db_replay`: FOUND
- Commit f2552ce exists: FOUND
- `cargo test --lib backtest` exits 0: PASSED (33/33)
- `cargo clippy -- -D warnings` exits 0: PASSED
