---
phase: 02-shadow-mode
plan: 01
subsystem: execution
tags: [shadow-mode, cli, safety, guard]
dependency_graph:
  requires: []
  provides: [ShadowGuard, shadow-cli-flags]
  affects: [src/main.rs, src/execution/mod.rs]
tech_stack:
  added: [thiserror]
  patterns: [enum-as-guard, copy-capture-closure]
key_files:
  created:
    - src/execution/shadow_guard.rs
  modified:
    - src/main.rs
    - src/execution/mod.rs
    - Cargo.toml
decisions:
  - "ShadowGuard is a Copy enum (not a struct with bool) — no heap allocation, captures freely into closures"
  - "guard.submit gate placed at out-of-range check in watch loop — natural rebalance trigger point in Phase 2"
  - "is_shadow() marked #[allow(dead_code)] — API surface for downstream plans (02, 03, 04)"
  - "ShadowGuardError re-exported with #[allow(unused_imports)] — available for callers in later plans"
metrics:
  duration: "5m 47s"
  completed: "2026-04-09"
  tasks_completed: 2
  files_changed: 4
requirements: [SHADOW-01]
---

# Phase 2 Plan 01: Shadow Guard + CLI Flags Summary

ShadowGuard enum with Copy semantics gates all transaction submission in shadow mode; `--shadow`/`--live` CLI flags added with clap mutual exclusion and shadow as default.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Add --shadow/--live flags to watch subcommand | 61838d1 | src/main.rs |
| 2 | Implement ShadowGuard and gate submission path | 6030e0f | src/execution/shadow_guard.rs, src/execution/mod.rs, src/main.rs, Cargo.toml |

## What Was Built

### RunMode enum (src/main.rs)
`RunMode { Shadow, Live }` derived from CLI flags after parse. Shadow is the default when neither flag is supplied. Logged at INFO level via `tracing::info!(mode = ?run_mode, "watch starting")`.

### ShadowGuard (src/execution/shadow_guard.rs)
- `ShadowGuard::shadow()` — blocks `submit()` with `ShadowGuardError::Blocked`, logs WARN
- `ShadowGuard::live()` — allows `submit()` with `Ok(())`, logs INFO
- `is_shadow()` — predicate for downstream plans
- 3 unit tests: shadow_blocks_submit, live_allows_submit, is_shadow_matches_constructor

### Submission gate in watch loop (src/main.rs)
When the position is out of range, `guard.submit(&plan_proxy)` is called before any action. In Phase 2 this uses a formatted string proxy; real `RebalancePlan` wiring arrives in Phase 5.

## Verification Results

- `cargo run -- watch --help` shows both `--shadow` and `--live` flags
- `cargo run -- watch --shadow --live` fails with clap conflict error
- `cargo test shadow_guard` — 3/3 tests pass
- `cargo clippy -- -D warnings` — exits 0

## Deviations from Plan

None — plan executed exactly as written.

## Threat Model Coverage

| Threat | Mitigation Applied |
|--------|-------------------|
| T-02-01: --live flag default | Shadow is default; `--live` requires explicit flag |
| T-02-02: ShadowGuard::submit bypass | Single entry point; unit tests assert Blocked in Shadow |
| T-02-03: No audit trail of mode choice | `tracing::info!(mode = ?run_mode)` logged at watch start |

## Known Stubs

None that affect this plan's goal. The `plan_proxy` string in the guard.submit call is intentional — real `RebalancePlan` struct wiring is Phase 5 scope.

## Self-Check: PASSED
