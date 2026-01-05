---
phase: 01-persistence
plan: 02
subsystem: database
tags: [postgres, timescaledb, sqlx, tokio, tracing, pnl, fire-and-forget]

# Dependency graph
requires:
  - phase: 01-persistence/01-01
    provides: storage::writer module with PoolTick + write_pool_tick foundation

provides:
  - PnlSnapshot struct (mint, pool_address, fees_earned, il_usd, net_pnl, position_value, price, observed_at)
  - write_pnl_snapshot async fn — awaitable DB insert into pnl_history
  - spawn_pnl_write fn — fire-and-forget tokio::spawn variant for non-blocking watch loop
  - pnl_history schema updated to PERSIST-02 columns (fees_earned, net_pnl, pool_address, position_value)

affects: [01-persistence/01-03, strategy, watch-loop, shadow-mode]

# Tech tracking
tech-stack:
  added: [tokio::task::JoinHandle, tracing::warn]
  patterns: [fire-and-forget spawn pattern for non-blocking DB writes, non-macro sqlx query path]

key-files:
  created: []
  modified:
    - src/storage/writer.rs
    - src/storage/schema.sql
    - src/storage/positions.rs

key-decisions:
  - "Renamed pnl_history columns fees_usd→fees_earned and net_usd→net_pnl to match PERSIST-02 domain language"
  - "record_pnl in positions.rs updated to use new column names with placeholder values (pool_address='', position_value=0.0) and marked TODO for back-compat; watch loop will use spawn_pnl_write instead"
  - "spawn_pnl_write returns JoinHandle<()> rather than being fully detached so callers can optionally join on shutdown"
  - "All SQL values bound as parameters — no string interpolation — satisfying T-01-07 (SQL injection)"
  - "Failures in spawn_pnl_write logged via tracing::warn! with mint context satisfying T-01-06 (repudiation)"

patterns-established:
  - "Non-blocking DB write pattern: spawn_pnl_write(pool, snap) returns JoinHandle immediately; errors logged not panicked"
  - "Non-macro sqlx query path: query() + .bind() + pool.execute() compiles without DATABASE_URL at build time"

requirements-completed: [PERSIST-02, PERSIST-03]

# Metrics
duration: 15min
completed: 2026-04-09
---

# Phase 01 Plan 02: PnlSnapshot Writer Summary

**pnl_history writer with fire-and-forget spawn_pnl_write enabling non-blocking P&L recording per tick event (PERSIST-02, PERSIST-03)**

## Performance

- **Duration:** ~15 min
- **Started:** 2026-04-09T00:00:00Z
- **Completed:** 2026-04-09T00:15:00Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments

- Extended `pnl_history` schema with PERSIST-02 columns: `fees_earned`, `net_pnl`, `pool_address`, `position_value`
- Added `PnlSnapshot` struct and `write_pnl_snapshot` async fn to `storage::writer`
- Added `spawn_pnl_write` fire-and-forget variant using `tokio::spawn`; failures logged via `tracing::warn!`
- Updated `positions.rs::record_pnl` INSERT to match new schema columns with legacy TODO marker

## Task Commits

Each task was committed atomically:

1. **Task 1: Extend pnl_history schema with PERSIST-02 columns** - `b8a2ca6` (feat)
2. **Task 2: Add PnlSnapshot + write_pnl_snapshot + spawn_pnl_write** - `624aee2` (feat)

## Files Created/Modified

- `src/storage/schema.sql` - pnl_history table updated: fees_usd→fees_earned, net_usd→net_pnl, added pool_address and position_value columns
- `src/storage/writer.rs` - Added PnlSnapshot struct, write_pnl_snapshot, spawn_pnl_write, and unit test pnl_snapshot_fields_accessible
- `src/storage/positions.rs` - Updated record_pnl INSERT to use new column names; added TODO deprecation comment

## Decisions Made

- Renamed `fees_usd`→`fees_earned` and `net_usd`→`net_pnl` to match PERSIST-02 domain language (breaking rename on empty/test DB only — no migration needed at this stage).
- `record_pnl` in `positions.rs` kept with same Rust signature but updated INSERT SQL to use new column names. Placeholder values `pool_address = ''` and `position_value = 0.0` ensure it compiles; TODO comment directs new code to `spawn_pnl_write`.
- `spawn_pnl_write` returns `JoinHandle<()>` (not detached) so callers can join on graceful shutdown if needed.
- All SQL values bound as parameters — satisfies threat T-01-07 (SQL injection via mint/pool_address).
- `tracing::warn!` on failure with `mint` context satisfies T-01-06 (repudiation / dropped write auditability).

## Deviations from Plan

None — plan executed exactly as written. The `record_pnl` update in `positions.rs` was explicitly specified in the Task 1 action block.

## Known Stubs

- `src/storage/positions.rs` line 76: `record_pnl` marked TODO for legacy back-compat. Intentional — per plan specification. New code uses `spawn_pnl_write`. Will be resolved when the watch loop is wired in shadow mode (plan 01-03 or later).

## Issues Encountered

None.

## User Setup Required

None — no external service configuration required beyond the DATABASE_URL already needed for plan 01-01.

## Next Phase Readiness

- `spawn_pnl_write(pool, snap)` is ready for plan 01-03 to wire into the watch loop
- `write_pnl_snapshot` is ready for integration tests once a live TimescaleDB instance is available
- Both `PoolTick` and `PnlSnapshot` writers are in `storage::writer` — plan 01-03 can import both for the full shadow-mode pipeline

---
*Phase: 01-persistence*
*Completed: 2026-04-09*

## Self-Check: PASSED

| Item | Status |
|------|--------|
| src/storage/writer.rs | FOUND |
| src/storage/schema.sql | FOUND |
| src/storage/positions.rs | FOUND |
| .planning/phases/01-persistence/01-02-SUMMARY.md | FOUND |
| Commit b8a2ca6 (Task 1) | FOUND |
| Commit 624aee2 (Task 2) | FOUND |
