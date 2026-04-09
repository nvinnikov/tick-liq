---
phase: 01-persistence
plan: "01"
subsystem: storage
tags: [persistence, pool-ticks, sqlx, timescaledb, schema]
dependency_graph:
  requires: []
  provides: [storage::writer::PoolTick, storage::writer::write_pool_tick, pool_ticks schema]
  affects: [phase-02-shadow-mode, phase-03-real-data-backtest]
tech_stack:
  added: []
  patterns: [non-macro sqlx query path, u128-to-decimal-string for NUMERIC(80,0), ON CONFLICT DO NOTHING idempotent upsert]
key_files:
  created:
    - src/storage/writer.rs
  modified:
    - src/storage/schema.sql
    - src/storage/mod.rs
key_decisions:
  - "Serialise u128 values as decimal strings with ::numeric cast rather than rust_decimal/BigDecimal to avoid extra dependencies"
  - "Use NUMERIC(80,0) for sqrt_price/liquidity/fee_growth to preserve full u128 precision without floating-point loss"
  - "Keep #[allow(dead_code)] and placeholder integration test stubs (matching positions.rs pattern) until watch loop is wired"
metrics:
  duration: "~3 minutes"
  completed_date: "2026-04-09"
  tasks_completed: 2
  files_modified: 3
requirements: [PERSIST-01, PERSIST-04]
---

# Phase 01 Plan 01: Pool Ticks Writer Summary

One-liner: Slot-keyed upsert writer for pool_ticks with NUMERIC(80,0) u128 columns and ON CONFLICT idempotency.

## What Was Built

### Task 1 — Schema update (commit `5d01614`)

Rewrote the `pool_ticks` table in `src/storage/schema.sql` to add the full WebSocket snapshot shape required by PERSIST-01:

| Column | Type | Notes |
|--------|------|-------|
| time | TIMESTAMPTZ NOT NULL | observation timestamp |
| pool_address | TEXT NOT NULL | pool pubkey |
| slot | BIGINT NOT NULL | Solana slot number |
| tick_current | INT NOT NULL | current active tick index |
| sqrt_price | NUMERIC(80,0) NOT NULL | pool sqrt price as Q64.64 integer |
| liquidity | NUMERIC(80,0) NOT NULL | active liquidity |
| fee_growth_global_a | NUMERIC(80,0) NOT NULL | cumulative fee growth token A |
| fee_growth_global_b | NUMERIC(80,0) NOT NULL | cumulative fee growth token B |
| UNIQUE (pool_address, slot) | — | idempotency key (PERSIST-04) |

The old `tick_index / liquidity_net` columns were removed; dev databases are reset rather than altered.

### Task 2 — storage::writer module (commit `c77339e`)

`src/storage/writer.rs` exports:

```rust
pub struct PoolTick {
    pub pool_address: String,
    pub slot: i64,
    pub tick_current: i32,
    pub sqrt_price: u128,
    pub liquidity: u128,
    pub fee_growth_global_a: u128,
    pub fee_growth_global_b: u128,
    pub observed_at: DateTime<Utc>,
}

pub async fn write_pool_tick(pool: &PgPool, tick: &PoolTick) -> Result<()>
```

Implementation details:
- u128 fields serialised as decimal strings; Postgres casts via `$N::numeric`
- Non-macro `sqlx_core::query::query` path — crate builds without `DATABASE_URL`
- `ON CONFLICT (pool_address, slot) DO NOTHING` — duplicate WebSocket deliveries are silently ignored
- All errors propagated via `anyhow::Context`; zero `unwrap()` calls
- `pub mod writer;` added to `src/storage/mod.rs`

## Deviations from Plan

None — plan executed exactly as written.

## Known Stubs

Integration test bodies in `writer.rs` are placeholder `#[ignore]` stubs requiring a live PostgreSQL/TimescaleDB instance. This matches the existing pattern in `positions.rs` and does not prevent the plan's goal (write path is fully implemented and compiles). Tests will be wired when the watch loop and a CI DB container are available.

## Threat Flags

No new security surfaces beyond the plan's threat model. All four STRIDE entries were addressed:
- T-01-01: All values passed via `.bind()` — no string interpolation into SQL.
- T-01-04: UNIQUE (pool_address, slot) + ON CONFLICT DO NOTHING prevents duplicate rows.

## Self-Check: PASSED

- src/storage/writer.rs — FOUND
- src/storage/schema.sql — FOUND (contains UNIQUE constraint, tick_current, fee_growth columns)
- src/storage/mod.rs — FOUND (contains `pub mod writer`)
- Commit 5d01614 — FOUND
- Commit c77339e — FOUND
- `cargo build` — PASSED
- `cargo clippy -- -D warnings` — PASSED
- `grep "unwrap()" src/storage/writer.rs` — no matches
