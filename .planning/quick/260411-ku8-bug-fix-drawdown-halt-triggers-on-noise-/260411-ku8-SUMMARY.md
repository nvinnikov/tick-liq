---
phase: quick-260411-ku8
plan: "01"
subsystem: strategy/risk_monitor
tags: [bug-fix, risk, drawdown, shadow-mode]
dependency_graph:
  requires: []
  provides: [drawdown-noise-guard]
  affects: [src/strategy/risk_monitor.rs]
tech_stack:
  added: []
  patterns: [threshold-constant-in-hot-path]
key_files:
  modified:
    - src/strategy/risk_monitor.rs
decisions:
  - "MIN_PEAK_USD = 1.0 chosen as noise threshold; sub-$1 peak values from early fee accruals are not meaningful for drawdown calculation"
metrics:
  duration: "~10m"
  completed: "2026-04-11"
  tasks_completed: 1
  files_modified: 1
---

# Phase quick-260411-ku8 Plan 01: Drawdown Halt Noise Fix Summary

**One-liner:** Added MIN_PEAK_USD = 1.0 guard in evaluate() so sub-dollar noise peaks no longer trigger false drawdown halts in shadow mode.

## What Was Done

In `RiskMonitor::evaluate()`, the drawdown check previously used `peak_pnl > 0.0` as the gate condition. In shadow mode, `peak_pnl` can be set to micro values (e.g., $0.0002) from fee accruals before the first WebSocket price update arrives. Any subsequent small price movement would then produce a massive drawdown percentage (e.g., 76–124%), immediately triggering `HaltAll` and making `--max-drawdown` unusable at startup.

**Fix:** Introduced `const MIN_PEAK_USD: f64 = 1.0` inside the drawdown block and changed the guard to `peak_pnl >= MIN_PEAK_USD`. Peaks below $1.00 are treated as noise and the drawdown check is skipped.

**Tests updated:**
- Renamed `drawdown_skipped_when_peak_not_positive` → `drawdown_skipped_when_peak_below_threshold`
- Kept the existing `peak=0.0` case
- Added `peak=0.5` case (formerly caused false halt: drawdown ~124%) — now returns `Continue` with `halt_flag=false`
- `drawdown_breach_returns_halt_all` (peak=100.0) unchanged and still passes, confirming real drawdown detection is unaffected

## Commits

| Task | Commit | Description |
|------|--------|-------------|
| 1    | a27f424 | fix(quick-260411-ku8): guard drawdown check with MIN_PEAK_USD threshold |

## Test Results

All 20 drawdown-related tests passed. No clippy issues in `risk_monitor.rs`.

## Deviations from Plan

None — plan executed exactly as written.

## Known Stubs

None.

## Threat Flags

None — change is internal to risk evaluation logic, no new network surface introduced.

## Self-Check: PASSED

- `src/strategy/risk_monitor.rs` modified with MIN_PEAK_USD constant
- Commit a27f424 exists
- All 20 drawdown tests pass
