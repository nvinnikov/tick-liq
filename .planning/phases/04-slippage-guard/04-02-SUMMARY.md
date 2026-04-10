---
plan: 04-02
phase: 04-slippage-guard
subsystem: strategy/execution
tags: [slippage, watch-loop, shadow-rebalance, gate]
dependency_graph:
  requires: [04-01]
  provides: [slippage-gate-wired]
  affects: [src/main.rs]
tech_stack:
  added: []
  patterns: [guard-flag-pattern, match-arm-with-guard]
key_files:
  modified:
    - src/main.rs
decisions:
  - "Used slippage_passed bool flag to gate build_rebalance_plan; avoids nested if/else restructuring of existing match block"
  - "Added wildcard Rebalance arm (without guard) to match to handle slippage-aborted case cleanly — returns None (abort row already written above)"
  - "Shadow guard submit moved inside slippage_passed block so simulation also respects gate"
metrics:
  duration: "8 minutes"
  completed: "2026-04-10T08:48:56Z"
  tasks_completed: 1
  files_modified: 1
requirements: [SLIPPAGE-01, SLIPPAGE-02]
---

# Phase 04 Plan 02: Wire slippage gate into watch loop and log abort to DB Summary

Slippage gate wired between `should_rebalance()` returning `Rebalance` and `build_rebalance_plan()` in the watch loop. High-impact rebalances are now aborted, warned, and persisted to `shadow_rebalances` with `trigger_reason='slippage_abort'`.

## What Was Built

Single task: modified `src/main.rs` to insert the slippage gate at the correct decision point.

**New control flow:**
```
should_rebalance() -> if Rebalance -> check_slippage() -> Ok: info log, slippage_passed=true
                                                       -> Abort: warn log, write abort row, slippage_passed=false
                   -> if slippage_passed -> shadow guard submit -> build_rebalance_plan() -> write shadow row
```

**Changes made:**

1. Added `let slippage_config = strategy::SlippageConfig::default();` before the rebalance decision block (Plan 04-03 will replace with CLI value).

2. Inserted slippage gate block after `is_rebalance` is set:
   - Calls `strategy::check_slippage(computed_position_value, price_current, pool.liquidity, &slippage_config)`
   - `Ok` arm: `tracing::info!` with `impact_bps`, `threshold_bps`, `position_value_usd`; sets `slippage_passed = true`
   - `Abort` arm: `tracing::warn!` with `impact_bps`, `threshold_bps`, `position_value_usd`; writes `ShadowRebalanceRow` with `trigger_reason="slippage_abort"`, NULL simulated fields, `error_flag: false`

3. Moved shadow guard `submit()` call inside `if slippage_passed` block (was `if is_rebalance`).

4. Guarded `build_rebalance_plan()` match arm with `if slippage_passed` guard pattern.

5. Added catch-all `Rebalance` arm (without guard) that returns `None` — handles slippage-aborted case where abort row was already written.

## Commits

| Task | Description | Hash | Files |
|------|-------------|------|-------|
| 04-02-T01 | Wire slippage gate into watch loop | e7d51bc | src/main.rs |

## Verification

- `cargo build` exits 0 (1 pre-existing external crate warning, unrelated)
- `cargo clippy -- -D warnings` exits 0
- `grep check_slippage src/main.rs` — 1 call found at line 642
- `grep slippage_abort src/main.rs` — 1 occurrence found at line 668
- `grep SlippageConfig src/main.rs` — construction at line 621
- `grep "SlippageResult::Abort" src/main.rs` — match arm at line 658

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing coverage] Added catch-all Rebalance arm to match block**

- **Found during:** Task 1 implementation
- **Issue:** Adding `if slippage_passed` guard to the `Rebalance { reason }` arm leaves the case where `is_rebalance=true` but `slippage_passed=false` unmatched — Rust requires exhaustive matches.
- **Fix:** Added a second `Ok(strategy::RebalanceDecision::Rebalance { .. })` arm (without guard) returning `None`, with a comment explaining the abort row was already written by the slippage gate above.
- **Files modified:** src/main.rs
- **Commit:** e7d51bc (included in task commit)

## Known Stubs

None — all wiring is functional. `SlippageConfig::default()` uses `max_bps: 50`; CLI override is intentionally deferred to Plan 04-03.

## Threat Flags

None — no new network endpoints, auth paths, or trust boundaries introduced. The slippage gate is a pure decision-path guard with no external I/O beyond the existing `shadow_rebalances` DB write.

## Self-Check: PASSED

- [x] `src/main.rs` modified and committed at e7d51bc
- [x] `cargo build` exits 0
- [x] `cargo clippy -- -D warnings` exits 0
- [x] All acceptance criteria met
