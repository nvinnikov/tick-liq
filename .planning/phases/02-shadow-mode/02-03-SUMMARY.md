---
phase: 02-shadow-mode
plan: 03
subsystem: storage
tags: [shadow-mode, gate, live-mode, safety]
dependency_graph:
  requires: [02-01, 02-02]
  provides: [check_shadow_gate, GateStatus, shadow-gate-enforcement]
  affects: [src/storage/writer.rs, src/main.rs]
tech_stack:
  added: []
  patterns: [fail-fast-exit, parameterised-query-scalar, gate-before-loop]
key_files:
  created: []
  modified:
    - src/storage/writer.rs
    - src/main.rs
decisions:
  - "No-DB + --live exits with code 2 rather than silently running gateless — prevents accidental live run without persistence"
  - "Gate uses sqlx_core::query_scalar matching existing codebase pattern (not sqlx macro path, no DATABASE_URL required at build time)"
  - "Two exit(2) call sites: no-DB case and GateStatus failure — both print hint directing operator to shadow mode"
metrics:
  duration: "~15m"
  completed: "2026-04-09"
  tasks_completed: 2
  files_changed: 2
requirements: [SHADOW-04]
---

# Phase 2 Plan 03: Shadow DB Gate Summary

Hard DB gate implemented in `check_shadow_gate`; `--live` startup blocked with exit code 2 unless pool has ≥14 days of shadow data with zero error rows.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Implement check_shadow_gate query | d590d35 | src/storage/writer.rs |
| 2 | Enforce gate at watch --live startup | 74a8380 | src/main.rs |

## What Was Built

### GateStatus enum (src/storage/writer.rs)

`GateStatus { Pass, NoData { pool_address }, TooRecent { earliest, required_age_days }, ErrorsPresent { count } }` with:
- `is_pass()` predicate
- `describe()` returning actionable error strings for each variant
- `SHADOW_GATE_REQUIRED_DAYS: i64 = 14` public constant

### check_shadow_gate (src/storage/writer.rs)

Async function taking `&PgPool` and `pool_address: &str`. Checks in precedence order:
1. `MIN(created_at)` query → `NoData` if NULL
2. Age check: `Utc::now() - earliest < Duration::days(14)` → `TooRecent`
3. `COUNT(*) WHERE error_flag = true` → `ErrorsPresent`
4. All passed → `GateStatus::Pass`

Uses `sqlx_core::query_scalar` (non-macro path, no DATABASE_URL at build time, matching existing codebase pattern).

### Gate enforcement in main.rs

Inserted between DB pool construction and `watch_account` call:
```
if matches!(run_mode, RunMode::Live) {
    match &db_pool {
        None => { eprintln!(...); std::process::exit(2); }
        Some(pg) => { check_shadow_gate(pg, &pool_addr).await? → exit 2 on failure }
    }
}
```
Shadow mode path is never gated. Gate runs per `pool_addr` from the fetched position.

### Unit tests (3 passing)

- `describe_failures_are_actionable` — NoData and ErrorsPresent describe strings contain expected text
- `gate_status_is_pass_predicate` — all four variants tested for is_pass()
- `too_recent_describe_contains_rfc3339_and_days` — TooRecent describe contains date and day count

## Verification Results

- `cargo test gate_tests` — 3/3 pass
- `cargo build` — exits 0
- `cargo clippy -- -D warnings` — exits 0
- `grep -n "RunMode::Live" src/main.rs` — 3 matches (declaration, guard arm, gate check)
- `grep -n "check_shadow_gate" src/main.rs` — 1 match, gated behind `RunMode::Live`
- `grep -n "std::process::exit(2)" src/main.rs` — 2 matches
- `grep -n "status.describe" src/main.rs` — 1 match

## Deviations from Plan

### Auto-adapted Implementation

**1. [Rule 2 - Missing critical functionality] Added no-DB + --live exit gate**

- **Found during:** Task 2
- **Issue:** Plan only handles `check_shadow_gate(&db_pool, ...)` where `db_pool: &PgPool`. In the actual code, `db_pool` is `Option<PgPool>` — if `--live` is passed without `DATABASE_URL`, the plan's code would panic or silently skip the gate.
- **Fix:** Added an explicit `None => exit(2)` branch before the `Some(pg)` branch. Operator gets a clear error pointing to shadow mode.
- **Files modified:** src/main.rs

**2. [Rule 1 - Adaptation] Used sqlx_core::query_scalar instead of sqlx::query_scalar**

- **Found during:** Task 1
- **Issue:** Plan snippet uses `sqlx::query_scalar(...)` macro form, but the project uses split `sqlx-core`/`sqlx-postgres` crates without the top-level `sqlx` crate or `DATABASE_URL`-required macros.
- **Fix:** Used `sqlx_core::query_scalar::query_scalar(...)` function path (same as `positions.rs`), which compiles without a live database URL.
- **Files modified:** src/storage/writer.rs

## Threat Model Coverage

| Threat | Mitigation Applied |
|--------|-------------------|
| T-02-08: Bypass gate via env var / config | Gate is code-enforced; no override flag; pool_address comes from fetched position (not user-supplied string). No-DB case also blocked. |
| T-02-09: Clock skew shortcuts 14-day window | Server DEFAULT NOW() for created_at; client Utc::now() for comparison. ~seconds skew is negligible for 14-day window. |
| T-02-10: Deleting error rows to pass gate | Out of scope: operator-owned DB. |
| T-02-11: DoS — gate query slow on large tables | idx_shadow_rebalances_pool_error partial index from Plan 02 covers the error_flag=true count query. |

## Known Stubs

None. Gate function is complete for its intended purpose. DB integration test is deferred to Plan 04 per plan specification.

## Self-Check: PASSED
