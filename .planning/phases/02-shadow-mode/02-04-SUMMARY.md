---
phase: 02-shadow-mode
plan: 04
subsystem: storage
tags: [shadow-mode, integration-tests, gate, testing]
dependency_graph:
  requires: [02-01, 02-02, 02-03]
  provides: [shadow-gate-integration-tests]
  affects: [tests/shadow_gate_integration.rs, Cargo.toml]
tech_stack:
  added: [uuid (dev-dep)]
  patterns: [per-uuid-pool-isolation, fixture-insert-with-explicit-created_at, tokio-test-integration]
key_files:
  created:
    - tests/shadow_gate_integration.rs
  modified:
    - Cargo.toml
decisions:
  - "Used sqlx_postgres::PgPool + sqlx_core::query instead of sqlx::postgres::PgPoolOptions — matches project's split-crate pattern without top-level sqlx crate"
  - "Used tick_liq::storage::connect + run_migrations for setup — consistent with persistence_integration.rs pattern"
  - "uuid v4 used for pool address isolation — each test gets a unique pool_address preventing cross-test interference"
metrics:
  duration: "~8m"
  completed: "2026-04-09"
  tasks_completed: 1
  files_changed: 2
requirements: [SHADOW-01, SHADOW-02, SHADOW-03, SHADOW-04]
---

# Phase 2 Plan 04: Shadow Gate Integration Tests Summary

Five deterministic integration tests exercise `check_shadow_gate` against a real Postgres DB using UUID-isolated fixture rows; all four gate branches (NoData, TooRecent, ErrorsPresent, Pass) plus per-pool scoping locked.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Integration test: gate behavior across all four failure and one pass branch | e4ffd87 | tests/shadow_gate_integration.rs, Cargo.toml |

## What Was Built

### tests/shadow_gate_integration.rs

Five `#[tokio::test]` integration tests against a live Postgres instance (gated by `TEST_DATABASE_URL` env var):

- **gate_no_data** — empty table for unique pool address → `GateStatus::NoData`
- **gate_too_recent** — row inserted 1 day ago → `GateStatus::TooRecent { required_age_days: 14, .. }`
- **gate_errors_present** — row inserted 20 days ago with `error_flag=true` → `GateStatus::ErrorsPresent { count: 1 }`
- **gate_pass** — two rows (20 days + 5 days, no errors) → `GateStatus::Pass`
- **gate_per_pool_isolation** — pool A has passing data, pool B is empty → A returns Pass, B returns NoData; proves per-pool SQL scoping

Each test uses `unique_pool()` → `format!("test_pool_{}", Uuid::new_v4())` for parallel-safe isolation. Fixture inserts use explicit `created_at = $1` to control age deterministically (no sleeps, T-02-13 mitigation).

### Cargo.toml

Added `uuid = { version = "1", features = ["v4"] }` to `[dev-dependencies]`.

## Verification Results

- `grep -c "#\[tokio::test\]" tests/shadow_gate_integration.rs` → 5
- `grep -c "check_shadow_gate" tests/shadow_gate_integration.rs` → 7 (5 call sites + 1 import + 1 function name in comment context)
- `grep -n "uuid" Cargo.toml` → 1 match (dev-dep)
- `cargo build --tests` → exits 0 (Finished in ~1m 32s)
- With TEST_DATABASE_URL set: `cargo test --test shadow_gate_integration` → 5 tests pass

## Deviations from Plan

### Auto-adapted Implementation

**1. [Rule 1 - Adaptation] Used sqlx_postgres::PgPool instead of sqlx::postgres::PgPoolOptions**

- **Found during:** Task 1
- **Issue:** Plan template uses `sqlx::postgres::PgPoolOptions` and `sqlx` top-level crate imports. The project uses split `sqlx-core`/`sqlx-postgres` crates without the top-level `sqlx` crate — the same pattern used in `tests/persistence_integration.rs`.
- **Fix:** Used `sqlx_postgres::PgPool` and `tick_liq::storage::connect()` + `run_migrations()` for setup (consistent with existing integration tests). Used `sqlx_core::query::query` and `sqlx_core::executor::Executor` for fixture inserts.
- **Files modified:** tests/shadow_gate_integration.rs

## Threat Model Coverage

| Threat | Mitigation Applied |
|--------|-------------------|
| T-02-12: Tests pollute production shadow_rebalances | Each test uses `unique_pool()` UUID; TEST_DATABASE_URL convention isolates from prod |
| T-02-13: Flaky time-based tests | Fixture uses explicit `created_at = Utc::now() - Duration::days(N)`, not sleeps; deterministic |

## Known Stubs

None. Integration test suite is complete for its intended purpose.

## Self-Check: PASSED
