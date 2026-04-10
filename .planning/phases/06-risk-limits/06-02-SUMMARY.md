---
phase: 06-risk-limits
plan: 02
subsystem: strategy
tags: [risk, risk-monitor, db-persistence, drift, rpc, borsh]

requires:
  - phase: 06-risk-limits
    plan: 01
    provides: RiskMonitor struct, RiskState struct, RiskAction enum, evaluate() method

provides:
  - "risk_state table DDL in schema.sql (pool_address PRIMARY KEY, all required columns)"
  - "RiskMonitor::load_or_init() — SELECT-then-INSERT preserving halt_flag on restart (D-12)"
  - "RiskMonitor::persist_state() — fire-and-forget tokio::spawn upsert (RISK-04)"
  - "RiskMonitor::derive_drift_user_pda() — PDA from [user, authority, 0u16] seeds"
  - "RiskMonitor::fetch_drift_margin_ratio() — real synchronous RPC fetch (D-01)"
  - "drift_user_pubkey and rpc_url fields added to RiskMonitor struct"
  - "18 unit tests passing (15 from Plan 01 + 3 new for Task 2)"

affects:
  - 06-03 (watch-loop wiring calls load_or_init at startup, persist_state each tick, fetch_drift_margin_ratio)

tech-stack:
  added: []
  patterns:
    - "SELECT-then-INSERT pattern for load_or_init: avoids overwriting halt_flag=true on restart"
    - "Fire-and-forget tokio::spawn for persist_state — mirrors spawn_pnl_write in storage::writer"
    - "Synchronous RpcClient with 5s timeout for fetch_drift_margin_ratio (spawn_blocking caller in Plan 03)"
    - "Proxy margin ratio approximation from raw PerpPosition bytes: |quote_sum|/(|base_sum|+1)"

key-files:
  created:
    - (none)
  modified:
    - src/storage/schema.sql
    - src/strategy/risk_monitor.rs

key-decisions:
  - "load_or_init uses SELECT-then-INSERT (not upsert) to preserve halt_flag=true on restart (D-12, RESEARCH.md Pitfall 2)"
  - "persist_state includes halt_flag in upsert so breach detected in-memory is immediately durable"
  - "fetch_drift_margin_ratio is synchronous (not async) for spawn_blocking compatibility in Plan 03"
  - "Proxy margin ratio = |quote_sum|/(|base_sum|+1) from raw PerpPosition bytes — documented approximation, LIVE-02 for full oracle-aware calculation"
  - "RPC failure returns None (margin OK fallback) — never cascades to halt (D-03)"
  - "drift_user_pubkey=None short-circuits fetch immediately (shadow mode / no keypair, RESEARCH.md Pitfall 5)"

requirements-completed: [RISK-03, RISK-04]

duration: 6min
completed: 2026-04-10
---

# Phase 6 Plan 02: RiskState DB Persistence and Drift RPC Fetch Summary

**risk_state table with SELECT-then-INSERT startup load (preserving halt_flag), fire-and-forget upsert persist, and synchronous Drift User account RPC fetch with proxy margin ratio from raw PerpPosition bytes.**

## Performance

- **Duration:** 6 min
- **Started:** 2026-04-10T14:03:21Z
- **Completed:** 2026-04-10T14:09:21Z
- **Tasks:** 2 of 2
- **Files modified:** 2

## Accomplishments

### Task 1: risk_state table DDL + DB load/persist methods

- Appended `risk_state` table to `src/storage/schema.sql` with `pool_address TEXT PRIMARY KEY`, `peak_pnl`, `current_drawdown_pct`, `pause_flag`, `halt_flag`, `updated_at` columns
- Implemented `load_or_init()` as SELECT-then-INSERT: reads existing row (preserving `halt_flag=true` from previous session with `tracing::error!` warning), or inserts fresh default row with `ON CONFLICT DO NOTHING` guard
- Implemented `persist_state()` as fire-and-forget `tokio::spawn` upsert, mirroring `spawn_pnl_write` pattern from `storage::writer`
- Added `sqlx_core` imports (`Executor`, `query`, `Row`) and `sqlx_postgres::PgPool` to risk_monitor.rs

### Task 2: Drift User PDA derivation + RPC margin fetch

- Added `drift_user_pubkey: Option<Pubkey>` and `rpc_url: String` fields to `RiskMonitor`; updated `new()` signature with two new parameters
- Implemented `derive_drift_user_pda()`: deterministic PDA from `["user", authority_bytes, 0u16_le]` seeds against `dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH`
- Implemented `fetch_drift_margin_ratio()`: synchronous `RpcClient` with 5-second timeout; skips if `drift_user_pubkey=None` or `drift_min_margin_ratio=None`; computes proxy ratio from raw PerpPosition bytes at known stride; all errors return `None` with `warn!`
- Added 3 unit tests for Task 2 (pda non-zero/unique, fetch returns None on missing pubkey, fetch returns None on missing limit); live RPC test marked `#[ignore]`
- All 18 tests pass; `cargo clippy -- -D warnings` clean; `cargo build` succeeds

## Task Commits

1. **Task 1: Add risk_state table DDL and DB load/persist methods** — `477adda` (feat)
2. **Task 2: Implement Drift User account RPC fetch with proxy margin ratio** — `eff157d` (feat)

## Files Created/Modified

- `src/storage/schema.sql` — risk_state table DDL appended after shadow_rebalances indexes
- `src/strategy/risk_monitor.rs` — load_or_init(), persist_state(), derive_drift_user_pda(), fetch_drift_margin_ratio(); drift_user_pubkey/rpc_url fields; updated new(); 3 new tests

## Deviations from Plan

### Pre-existing fmt issue (out of scope)

**[Out of scope] src/backtest/db_replay.rs formatting**
- `cargo fmt --check` shows a diff in `src/backtest/db_replay.rs` (import order + line wrapping)
- This file was last modified by Plan 01 commit 9406ccc and is unrelated to Plan 02's changes
- Not fixed per deviation scope boundary: only auto-fix issues directly caused by the current task's changes
- Logged to deferred-items for Phase 06 cleanup

## Known Stubs

None — `load_or_init()` and `persist_state()` are complete implementations; `fetch_drift_margin_ratio()` is a real RPC call (D-01 satisfied). The proxy margin ratio is explicitly an approximation documented in code comments, not a stub — it produces real values from real account data.

## Threat Flags

| Flag | File | Description |
|------|------|-------------|
| threat_flag: T (Tampering) | src/strategy/risk_monitor.rs | T-06-04: Drift User account data validated (length > 8 before parse); parse errors return None — cannot trigger halt |
| threat_flag: D (DoS) | src/strategy/risk_monitor.rs | T-06-05: 5-second RpcClient timeout; fetch failure = None (skip check), never halts |
| threat_flag: T (Tampering) | src/storage/schema.sql | T-06-08: All DB operations use parameterized sqlx bind() — no string interpolation in SQL |

All mitigations from the plan's threat register are implemented as required.

## Self-Check: PASSED

- `src/storage/schema.sql` — contains `CREATE TABLE IF NOT EXISTS risk_state`, `pool_address TEXT PRIMARY KEY`, `peak_pnl DOUBLE PRECISION NOT NULL DEFAULT 0.0`, `halt_flag BOOLEAN NOT NULL DEFAULT FALSE`
- `src/strategy/risk_monitor.rs` — contains `pub async fn load_or_init`, `pub fn persist_state`, `tokio::spawn`, `ON CONFLICT (pool_address) DO UPDATE`, `halt_flag set from previous session`, `pub fn derive_drift_user_pda`, `pub fn fetch_drift_margin_ratio`, `dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH`, `get_account_data`, `drift user RPC fetch failed`, `approximation`, `rpc_url`, `drift_user_pubkey: Option<`
- Does NOT contain `ON CONFLICT DO UPDATE SET halt_flag = false` or `halt_flag = FALSE` in the load_or_init path
- Commits `477adda` and `eff157d` verified in git log
- All 18 tests pass: `cargo test --lib strategy::risk_monitor` exits 0
- `cargo clippy -- -D warnings` exits 0 (one pre-existing solana-client future-incompat warning from dependency, not from project code)
- `cargo build` succeeds
