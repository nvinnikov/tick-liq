---
phase: 03-real-data-backtest
plan: "03"
subsystem: cli
tags: [cli-wiring, backtest, db-mode, gbm-mode, clap, fixture-tests]
dependency_graph:
  requires:
    - storage::tick_reader::read_ticks (03-01)
    - backtest::db_replay::run_db_backtest (03-02)
    - storage::connect (existing)
    - backtest::print_results (existing shared display path)
  provides:
    - backtest subcommand --pool flag enabling DB-mode replay
    - CLI flags: --pool, --from, --to, --position-liquidity, --near-edge-ticks,
      --range-lower-factor, --range-upper-factor
  affects:
    - src/main.rs (Backtest command variant extended)
    - src/backtest/db_replay.rs (dead_code suppression removed)
    - src/storage/tick_reader.rs (struct-level dead_code annotation refined)
tech_stack:
  added: []
  patterns:
    - optional --pool flag gates DB vs GBM branch in a single Backtest variant
    - NaiveDate parsed from CLI string via str::parse() with anyhow error wrapping
    - shared backtest::print_results for both GBM and DB output paths
key_files:
  created: []
  modified:
    - src/main.rs
    - src/backtest/db_replay.rs
    - src/storage/tick_reader.rs
decisions:
  - "Single Backtest variant extended with optional DB flags rather than a separate BacktestDb subcommand — avoids duplication of shared args (entry-price, price-lower, price-upper, capital, tick-spacing, rebalance)"
  - "#[allow(dead_code)] moved from module-level to struct-level on PoolTickRow — fields are public row contract, partial field use by db_replay is intentional"
  - "position_liquidity is u64 at CLI (clap default_value_t requires Default+Display) and cast to u128 at the call site — avoids adding a custom parser"
  - "Empty tick result returns anyhow::bail with actionable message rather than panicking — matches CLAUDE.md no-unwrap rule"
metrics:
  duration: "~25 minutes"
  completed: "2026-04-09T21:20:43Z"
  tasks_completed: 1
  tasks_total: 1
  files_created: 0
  files_modified: 3
---

# Phase 03 Plan 03: CLI Wiring + Fixture Tests Summary

**One-liner:** DB-mode backtest wired into `backtest` CLI subcommand via optional `--pool` flag, sharing GBM display path; 4 fixture-based unit tests verify constant-price output invariants.

## What Was Built

### CLI Extension (`src/main.rs`)

The existing `Backtest` subcommand variant was extended with seven new optional flags:

| Flag | Type | Default | Purpose |
|------|------|---------|---------|
| `--pool <ADDR>` | `Option<String>` | None | Pool address; presence enables DB mode |
| `--from <YYYY-MM-DD>` | `Option<String>` | None | Inclusive start date for DB replay |
| `--to <YYYY-MM-DD>` | `Option<String>` | None | Exclusive end date for DB replay |
| `--position-liquidity <u64>` | `u64` | 0 | Liquidity units held (DB mode) |
| `--near-edge-ticks <i32>` | `i32` | 0 (off) | Near-edge rebalance trigger (DB mode) |
| `--range-lower-factor <f64>` | `f64` | 0.95 | Lower range width factor on rebalance |
| `--range-upper-factor <f64>` | `f64` | 1.05 | Upper range width factor on rebalance |

**Dispatch logic in `Commands::Backtest` match arm:**

- `--pool Some(addr)` → DB mode:
  1. Validates `--db-url` / `DATABASE_URL` is set (actionable error if not)
  2. Parses `--from` and `--to` as `NaiveDate` with clear errors on missing/invalid input
  3. Validates `to > from`
  4. Connects to Postgres via `storage::connect()`
  5. Calls `storage::tick_reader::read_ticks()` for the pool + date range
  6. Returns actionable error if no ticks found (tells user to run `watch` first)
  7. Constructs `DbBacktestInput` from shared flags + DB-specific flags
  8. Calls `backtest::db_replay::run_db_backtest()`
  9. Prints tick count, then calls `backtest::print_results()` (shared with GBM)

- `--pool None` → GBM mode (unchanged): constructs `BacktestParams`, calls `backtest::run()`, then `backtest::print_results()`

### Dead-code annotation cleanup

- `src/backtest/db_replay.rs`: Removed `#![allow(dead_code)]` module-level suppressor — the module is now fully wired.
- `src/storage/tick_reader.rs`: Replaced `#![allow(dead_code)]` with `#[allow(dead_code)]` on the `PoolTickRow` struct — the struct is the complete row contract; `pool_address`, `slot`, `fee_growth_global_b` are not consumed by `db_replay` but are part of the public API for future callers.

### Fixture tests (`src/backtest/db_replay.rs`)

Four new unit tests added to the existing `backtest::db_replay::tests` module:

| Test | What it verifies |
|------|-----------------|
| `db_mode_in_range_constant_price_fees_positive` | Constant sqrt_price → IL=0, fees>0, all days in range, net_pnl=fees |
| `db_mode_net_pnl_equals_fees_plus_il_each_day` | `net_pnl_usd == fees + il` holds at every DayResult (mirrors GBM invariant) |
| `db_mode_fee_apy_is_non_negative_and_finite` | `fee_apy_pct >= 0` and `is_finite()` |
| `db_mode_params_snapshot_matches_input` | `ParamsSnapshot` fields populated from `DbBacktestInput`; GBM-only fields (`annual_vol_pct`, `daily_volume_usd`) are 0.0 |

Total test count: 117 (was 113). All pass.

## Example Usage

```bash
# GBM mode (unchanged)
cargo run -- backtest \
  --entry-price 1.0 --price-lower 0.90 --price-upper 1.10 \
  --days 30 --volatility 0.80

# DB mode — replay real ticks from TimescaleDB
DATABASE_URL=postgres://user:pass@localhost/tickliq \
cargo run -- backtest \
  --pool So1endKv3PFuvFDPHbDmAyREHFnpRQRoHhq9J7k2xJm \
  --from 2026-01-01 --to 2026-02-01 \
  --entry-price 1.0 --price-lower 0.90 --price-upper 1.10 \
  --capital 10000 --position-liquidity 1000000 \
  --rebalance --range-lower-factor 0.95 --range-upper-factor 1.05
```

## Verification

```
cargo clippy -- -D warnings   → 0 errors, 0 warnings (project-owned code)
cargo test --lib              → 117 passed, 0 failed, 8 ignored
```

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing critical] `#[allow(dead_code)]` on PoolTickRow struct**
- **Found during:** clippy run after removing module-level allow
- **Issue:** `pool_address`, `slot`, `fee_growth_global_b` fields reported as dead code since only `time`, `tick_current`, `sqrt_price`, `liquidity`, `fee_growth_global_a` are consumed by db_replay
- **Fix:** Moved allow from module level to the struct with doc-comment explaining intent (public row contract, partial use intentional)
- **Files modified:** src/storage/tick_reader.rs
- **Commit:** 3ddf2bd

**2. [Rule 2 - Missing critical] Validate date ordering before DB call**
- **Found during:** Task 1 implementation
- **Issue:** `to <= from` would produce an empty result without a clear error
- **Fix:** Added explicit `anyhow::bail!` check with message "–to must be after –from"
- **Files modified:** src/main.rs
- **Commit:** 3ddf2bd

**3. [Rule 2 - Missing critical] Actionable empty-ticks error**
- **Found during:** Task 1 implementation
- **Issue:** Passing an empty slice to `run_db_backtest` returns an anyhow error but the message was generic
- **Fix:** Added pre-call check with message directing user to run `watch` first if no ticks exist for the given pool + date range
- **Files modified:** src/main.rs
- **Commit:** 3ddf2bd

## Known Stubs

None. The DB-mode backtest path is fully functional end-to-end. Integration tests that require a live TimescaleDB remain `#[ignore]` (established in plans 03-01 and 03-02) — those are not stubs, they are correctly gated on a runtime dependency.

## Threat Flags

None. No new network endpoints, auth paths, file access patterns, or schema changes. The DB connection reuses `storage::connect()` which is already used by `db migrate` and `watch`. Pool address is passed as a parameterised bind in `read_ticks` (T-03-01 carries over from 03-01).

## Self-Check: PASSED

- [x] `src/main.rs` contains `--pool` optional arg and DB-mode dispatch branch: FOUND
- [x] `src/backtest/db_replay.rs` no longer contains `#![allow(dead_code)]`: CONFIRMED
- [x] `src/storage/tick_reader.rs` has `#[allow(dead_code)]` on struct: CONFIRMED
- [x] `cargo build` exits 0: PASSED
- [x] `cargo clippy -- -D warnings` exits 0: PASSED
- [x] `cargo test --lib` — 117 passed, 0 failed: PASSED
- [x] Commit 3ddf2bd exists: CONFIRMED
