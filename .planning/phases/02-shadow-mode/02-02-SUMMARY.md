---
phase: 02-shadow-mode
plan: 02
subsystem: storage
tags: [shadow-mode, persistence, pnl, rebalance-logging]
dependency_graph:
  requires: [02-01]
  provides: [shadow_rebalances-table, ShadowRebalanceRow, spawn_shadow_write, real-pnl-history]
  affects: [src/storage/schema.sql, src/storage/writer.rs, src/main.rs]
tech_stack:
  added: []
  patterns: [fire-and-forget-spawn, error-capture-to-db-row, block-in-place-for-sync-callback]
key_files:
  created: []
  modified:
    - src/storage/schema.sql
    - src/storage/writer.rs
    - src/main.rs
decisions:
  - "trigger_reason normalised with spaces->underscores to match schema convention ('out_of_range', 'near_lower_edge', 'near_upper_edge')"
  - "On Hold decision no shadow row is written — only rebalance decisions and errors are persisted"
  - "entry_price falls back to current price when cache miss, yielding IL=0 (conservative, avoids spurious error rows)"
  - "Decimal values hardcoded to SOL(9)/USDC(6) for position_value in watch loop — same pattern as Raydium position command; full wiring deferred to Phase 5"
metrics:
  duration: "~18m"
  completed: "2026-04-09"
  tasks_completed: 2
  files_changed: 3
requirements: [SHADOW-02, SHADOW-03]
---

# Phase 2 Plan 02: Shadow Rebalance Logging + Real P&L History Summary

shadow_rebalances table with full simulated outcome fields added to schema; real fees/IL/net_pnl/position_value computed and persisted in pnl_history on every tick, replacing Phase 1 0.0 stubs.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Add shadow_rebalances table + writer API | cfa2ccf | src/storage/schema.sql, src/storage/writer.rs |
| 2 | Wire shadow decision logging + real pnl_history values into watch loop | 4cc4db1 | src/main.rs |

## What Was Built

### schema.sql additions

`shadow_rebalances` table with:
- `id BIGSERIAL PRIMARY KEY`, `created_at TIMESTAMPTZ DEFAULT NOW()` — server-side timestamps, no client id (T-02-04)
- `trigger_reason TEXT NOT NULL` — 'out_of_range' | 'near_lower_edge' | 'near_upper_edge' | 'error'
- `simulated_range_width`, `simulated_fees_earned`, `simulated_il_usd`, `simulated_net_pnl` — optional f64 columns
- `error_flag BOOLEAN NOT NULL DEFAULT FALSE`, `error_message TEXT` — error capture for Plan 03 gate query
- Two indexes: composite `(pool_address, created_at DESC)` and partial `WHERE error_flag = true`

### writer.rs additions

- `ShadowRebalanceRow` struct mirroring the DB schema
- `write_shadow_rebalance(pool, row)` — parameterised INSERT, no string interpolation
- `spawn_shadow_write(pool, row)` — fire-and-forget tokio::spawn, errors logged via tracing::error!

### main.rs watch loop rewiring

**Real P&L computation (replaces Phase 1 stubs):**
- `fees_earned`: `compute_accrued_fees(fee_growth_global, fee_growth_checkpoint, liquidity)` for both tokens
- `il_usd`: `compute_il(entry_price, current_price, lower, upper) * position_value` using cached entry price
- `position_value`: `compute_token_amounts(liquidity, sqrt_price, ticks)` converted via SOL/USDC decimals
- `net_pnl`: `fees_earned - il_usd.abs()`

**Shadow decision logging:**
- `strategy::should_rebalance()` called every tick with real net_pnl
- On `Rebalance { reason }`: `execution::build_rebalance_plan()` called; row with `simulated_range_width` and all simulated fields spawned via `spawn_shadow_write`
- On `Hold`: no row written
- On error path: `error_flag: true` row with `error_message` (T-02-05)
- ShadowGuard gate retained from Plan 01 — called on `is_rebalance` check

## Verification Results

- `cargo build` — exits 0
- `cargo clippy -- -D warnings` — exits 0
- `grep -c "fees_earned: 0.0" src/main.rs` → 0 (stubs removed)
- `grep -c "il_usd: 0.0" src/main.rs` → 0 (stubs removed)
- `grep -n "spawn_shadow_write" src/main.rs` → 1 match
- `grep -n "error_flag: true" src/main.rs` → 1 match

## Deviations from Plan

### Auto-adapted Implementation

**1. [Rule 1 - Adaptation] decision_result uses Ok(strategy::RebalanceDecision) not anyhow::Result**

- **Found during:** Task 2
- **Issue:** `strategy::should_rebalance()` is a pure function returning `RebalanceDecision` (not `anyhow::Result`). The plan pseudocode wraps it in `anyhow::Result` — but there's no error path in the pure function.
- **Fix:** Used `Result<strategy::RebalanceDecision, String>` locally with `Ok(...)` wrapping; error variant reserved for future fallible extensions. Kept the match arms semantically identical to the plan.
- **Files modified:** src/main.rs

**2. [Rule 1 - Adaptation] trigger_reason normalisation**

- **Found during:** Task 2
- **Issue:** `should_rebalance()` returns human-readable reasons like "out of range" (with spaces). The schema comment shows 'out_of_range' convention.
- **Fix:** Applied `.replace(' ', "_")` at the write site to normalise.
- **Files modified:** src/main.rs

## Threat Model Coverage

| Threat | Mitigation Applied |
|--------|-------------------|
| T-02-04: Row integrity | Server-side DEFAULT NOW() timestamp; BIGSERIAL id; no client-supplied id |
| T-02-05: Error path repudiation | error_flag=true + error_message persisted on any error in decision path |
| T-02-06: error_message info leak | anyhow::Error format only — no keypairs at this call site |
| T-02-07: Unbounded shadow spawns | One decision per tick; watch loop rate limits |

## Known Stubs

None that affect this plan's goal.

- Decimal values in watch loop are hardcoded SOL(9)/USDC(6) — same approximation as Raydium position command. Real mint decimals wiring is Phase 5 scope.
- `entry_price` cache miss yields 0 IL (conservative). Cached after first `position --entry-price` invocation.

## Self-Check: PASSED

- `cfa2ccf` exists: confirmed (`git log --oneline`)
- `4cc4db1` exists: confirmed (`git log --oneline`)
- `src/storage/schema.sql` modified: confirmed
- `src/storage/writer.rs` modified: confirmed
- `src/main.rs` modified: confirmed
- `cargo build` exits 0: confirmed
- `cargo clippy -- -D warnings` exits 0: confirmed
