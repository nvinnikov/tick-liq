---
phase: 03-real-data-backtest
plan: "01"
subsystem: storage
tags: [tick-reader, sqlx, timescaledb, backtest, u128, numeric]
dependency_graph:
  requires:
    - src/storage/schema.sql (pool_ticks table definition)
    - src/storage/writer.rs (PoolTick struct — mirrored for PoolTickRow)
    - src/storage/mod.rs (module export pattern)
  provides:
    - storage::tick_reader::PoolTickRow (typed row struct)
    - storage::tick_reader::read_ticks (async paginated query)
  affects:
    - Phase 03 plan 02 (db_replay — consumes read_ticks as data source)
    - Phase 03 plan 03 (CLI wiring — reads ticks via tick_reader)
tech_stack:
  added: []
  patterns:
    - sqlx non-macro query pattern (no compile-time DATABASE_URL)
    - NUMERIC(80,0) → TEXT cast in SQL → u128::parse() in Rust
    - PgPool passed as &PgPool parameter (injection pattern)
key_files:
  created:
    - src/storage/tick_reader.rs
  modified:
    - src/storage/mod.rs
decisions:
  - "NUMERIC(80,0) cast to TEXT in SQL and parsed as u128 in Rust — sqlx-postgres has no native u128 codec; matches write path in writer.rs"
  - "#![allow(dead_code)] added following writer.rs pattern — module will be wired up in plan 03-02"
  - "Integration tests added as #[ignore] stubs — require live TimescaleDB; unit tests cover u128 parse and timestamp logic without DB"
metrics:
  duration: "~15 minutes"
  completed: "2026-04-09"
  tasks_completed: 1
  tasks_total: 1
  files_created: 1
  files_modified: 1
---

# Phase 03 Plan 01: tick_reader — DB Reader for pool_ticks Summary

**One-liner:** Async paginated reader for pool_ticks with NUMERIC(80,0)→u128 parsing, parameterised SQLi-safe query, and chronological ordering.

## What Was Built

`storage::tick_reader` provides the data access layer for the DB-mode backtest replay introduced in Phase 3.

- **`PoolTickRow`** struct mirrors `storage::writer::PoolTick`, using the `time` column name (matching the DB schema column name).
- **`read_ticks(pool, pool_address, from, to)`** queries `pool_ticks` for rows where `time >= from` (UTC midnight) and `time < to` (UTC midnight exclusive), ordered by `(time ASC, slot ASC)`. All four NUMERIC(80,0) columns are cast to TEXT in SQL and parsed as `u128` in Rust.
- Module exported from `storage/mod.rs` as `pub mod tick_reader`.

## Threat Mitigations Applied

| Threat | Mitigation |
|--------|-----------|
| T-03-01 SQLi via pool_address | Parameterised bind — no string interpolation into SQL |
| T-03-03 Error disclosure | anyhow::Context wraps errors with safe labels |
| T-03-04 Bad u128 parse | String::parse::<u128>() + .context() returns Err on bad row |
| T-03-02 DoS (large range) | Accepted — operator-controlled local CLI tool |

## Tests

- **Unit tests (run without DB):** u128 parsing from decimal string (2^64, MAX, 0, negative), UTC midnight timestamp derivation for from/to bounds.
- **Integration tests (ignored):** Roundtrip write+read, empty result, chronological ordering — stubs require live TimescaleDB with `DATABASE_URL`.
- All 4 unit tests pass: `cargo test storage::tick_reader`

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing critical] Added `#![allow(dead_code)]`**
- **Found during:** Task 1 — clippy -D warnings flagged PoolTickRow and read_ticks as dead code
- **Issue:** Module not yet consumed by any caller; clippy -D warnings treats unused pub items as errors
- **Fix:** Added `#![allow(dead_code)]` at file top, matching the identical pattern in `src/storage/writer.rs`
- **Files modified:** src/storage/tick_reader.rs
- **Commit:** 0c0c01f

## Known Stubs

None — tick_reader does not render data to UI. Integration test stubs are intentionally `#[ignore]` pending live DB.

## Threat Flags

None — no new network endpoints, auth paths, or trust boundaries beyond what the plan's threat model covers.

## Self-Check: PASSED

- [x] `src/storage/tick_reader.rs` exists and contains `pub struct PoolTickRow`, `pub async fn read_ticks`, `FROM pool_ticks`, `ORDER BY time ASC`
- [x] `src/storage/mod.rs` contains `pub mod tick_reader`
- [x] `cargo build` exits 0
- [x] `cargo clippy -- -D warnings` exits 0 (via rtk wrapper)
- [x] `cargo test storage::tick_reader` — 4 unit tests pass, 6 ignored
- [x] Commit 0c0c01f exists
