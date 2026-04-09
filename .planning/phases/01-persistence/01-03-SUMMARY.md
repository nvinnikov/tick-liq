---
phase: 01-persistence
plan: "03"
subsystem: storage
tags: [persistence, watch-loop, integration-test, postgres]
dependency_graph:
  requires: [01-01, 01-02]
  provides: [watch-loop-persistence, integration-test-suite]
  affects: [src/main.rs, src/storage/writer.rs, tests/persistence_integration.rs]
tech_stack:
  added: [src/lib.rs (lib target for test access)]
  patterns: [block_in_place for sync→async bridge, fire-and-forget tokio::spawn, ON CONFLICT DO NOTHING idempotency]
key_files:
  created:
    - src/lib.rs
    - src/storage/writer.rs (extended with PnlSnapshot + write_pnl_snapshot + spawn_pnl_write)
    - tests/persistence_integration.rs
  modified:
    - src/main.rs (watch command wired to storage)
    - src/storage/schema.sql (pool_ticks + pnl_history updated schemas)
    - src/storage/mod.rs (pub mod writer added)
    - src/storage/positions.rs (record_pnl INSERT updated for renamed columns)
    - Cargo.toml ([lib] target added)
decisions:
  - "Use tokio::task::block_in_place + Handle::current().block_on() to await write_pool_tick from sync WS callback — avoids architectural refactor of the callback type"
  - "Add src/lib.rs + [lib] Cargo target so integration tests can use tick_liq::storage paths"
  - "Extend writer.rs with PnlSnapshot/write_pnl_snapshot/spawn_pnl_write in this worktree (plan 01-02 runs in parallel worktree; orchestrator merges)"
metrics:
  duration_minutes: ~35
  completed: "2026-04-09"
  tasks_completed: 2
  files_changed: 8
---

# Phase 1 Plan 03: Watch Loop Persistence Wiring Summary

Wired `storage::writer::write_pool_tick` (awaited) and `storage::writer::spawn_pnl_write` (fire-and-forget) into the `watch` command event loop, plus an integration test suite covering both writers end-to-end.

## What Was Built

### Task 1: Wire writers into the watch command

**Wiring points in `src/main.rs`:**

1. **DB connect + migrate at startup** (lines added to `Commands::Watch` handler):
   ```rust
   let db_pool: Option<sqlx_postgres::PgPool> = match cli.db_url.as_deref() {
       Some(url) => { let pg = storage::connect(url).await?; storage::run_migrations(&pg).await?; Some(pg) }
       None => { tracing::warn!("DATABASE_URL not set — running watch without persistence"); None }
   };
   ```

2. **Per-tick persistence** (inside the `on_notify` closure, after pool data is fetched):
   - Extracts `slot` from WS JSON at `params.result.context.slot`
   - Constructs `PoolTick` from the parsed Orca Whirlpool pool struct fields
   - `write_pool_tick` called via `tokio::task::block_in_place` + `Handle::current().block_on()` — this is the durability checkpoint (awaited)
   - Constructs `PnlSnapshot` with `price` wired, all P&L fields as 0.0 placeholders (TODO phase-2)
   - `spawn_pnl_write` called via `std::mem::drop(spawn_pnl_write(...))` — fire-and-forget (PERSIST-03)

3. **Graceful degradation**: if `DATABASE_URL` is absent, watch runs normally with a `tracing::warn!` log and `db_pool = None`. No crash.

**Schema updates (`src/storage/schema.sql`):**
- `pool_ticks`: added `slot BIGINT`, `tick_current`, `sqrt_price NUMERIC(80,0)`, `liquidity NUMERIC(80,0)`, `fee_growth_global_a/b NUMERIC(80,0)`, `UNIQUE(pool_address, slot)`
- `pnl_history`: added `pool_address`, renamed `fees_usd→fees_earned`, `net_usd→net_pnl`, added `position_value`

**`src/storage/writer.rs`** (complete — covers plans 01-01 and 01-02 output):
- `PoolTick` struct + `write_pool_tick` (ON CONFLICT DO NOTHING, PERSIST-04)
- `PnlSnapshot` struct + `write_pnl_snapshot` + `spawn_pnl_write` (PERSIST-02, PERSIST-03)

### Task 2: Integration test (`tests/persistence_integration.rs`)

Three tests, all gated with `#[ignore = "requires live PostgreSQL/TimescaleDB"]`:

| Test | What it verifies |
|------|-----------------|
| `pool_tick_write_is_idempotent` | Same (pool_address, slot) inserted twice → COUNT = 1 (PERSIST-04) |
| `pnl_snapshot_write_persists` | PnlSnapshot written → fees_earned queryable from pnl_history (PERSIST-02) |
| `spawn_pnl_write_is_non_blocking` | 50 spawns complete < 100ms wall-clock (PERSIST-03) |

**Run command:**
```bash
DATABASE_URL=postgres://user:pass@localhost/tickliq \
  cargo test --test persistence_integration -- --ignored
```

## Known Stubs

| File | Location | Value | Reason |
|------|----------|-------|--------|
| `src/main.rs` | watch closure, PnlSnapshot | `fees_earned: 0.0` | Strategy layer not yet landed (Phase 2) |
| `src/main.rs` | watch closure, PnlSnapshot | `il_usd: 0.0` | IL requires position entry price wiring (Phase 2) |
| `src/main.rs` | watch closure, PnlSnapshot | `net_pnl: 0.0` | Derived from fees + IL (Phase 2) |
| `src/main.rs` | watch closure, PnlSnapshot | `position_value: 0.0` | Requires amounts × price computation (Phase 2) |

These stubs are intentional per plan spec ("use 0.0 placeholders...the point is the wiring, not the math"). The wiring path itself is fully functional — Phase 2 will replace the stubs.

## Architectural Decision: sync→async bridge

The WS callback type is `Box<dyn Fn(serde_json::Value) + Send + 'static>` (sync). `write_pool_tick` is async. Rather than refactoring the callback type (Rule 4 territory — breaking change to `data::ws`), used `tokio::task::block_in_place` + `Handle::current().block_on()` to call the async writer from within the sync closure while inside a multi-threaded tokio runtime. This is the standard Tokio pattern for this case.

## Deviations from Plan

### Auto-added: lib.rs + [lib] Cargo target (Rule 2)

The plan's integration test template uses `use tick_liq::storage::...`. The crate had only a binary target — no lib target means integration tests can't access internal modules via the crate name.

- **Fix**: added `src/lib.rs` re-exporting all modules, added `[lib]` entry to Cargo.toml
- **Files**: `src/lib.rs` (new), `Cargo.toml` (modified)
- **Commit**: `8e5de34`

### Auto-added: PnlSnapshot + spawn_pnl_write in this worktree (Rule 3)

Plan 01-02 runs in a parallel worktree and adds these to writer.rs. This worktree didn't have them, blocking compilation of the watch wiring. Added the full writer.rs implementation here; the orchestrator's merge will reconcile with 01-02's output.

- **Files**: `src/storage/writer.rs`
- **Commit**: `40f138e`

### Auto-fixed: positions.rs record_pnl column rename (Rule 1)

The `pnl_history` schema renamed `fees_usd→fees_earned` and `net_usd→net_pnl`, and added `pool_address` + `position_value`. The existing `record_pnl` INSERT referenced old column names and would fail at runtime against the new schema.

- **Fix**: updated INSERT to use new column names with placeholder values for the added columns
- **Files**: `src/storage/positions.rs`
- **Commit**: `40f138e`

## Threat Flags

None — no new network endpoints, auth paths, or trust boundaries introduced beyond those in the plan's threat model.

## Self-Check: PASSED

- `src/storage/writer.rs` exists: FOUND
- `tests/persistence_integration.rs` exists: FOUND
- `src/lib.rs` exists: FOUND
- Commit `40f138e` exists: FOUND
- Commit `8e5de34` exists: FOUND
- `cargo build` exits 0: PASSED
- `cargo clippy -- -D warnings` exits 0: PASSED
- `cargo build --tests` exits 0: PASSED
- Integration test list shows 3 tests: PASSED
