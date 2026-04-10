---
phase: 06-risk-limits
plan: 01
subsystem: strategy
tags: [risk, risk-monitor, drawdown, impermanent-loss, drift, state-machine]

requires:
  - phase: 01-persistence
    provides: PnlSnapshot struct in storage::writer used as evaluate() input

provides:
  - "RiskMonitor struct with evaluate() pure state machine"
  - "RiskState struct (pool_address, peak_pnl, current_drawdown_pct, pause_flag, halt_flag)"
  - "RiskAction enum (Continue, PauseRebalancing, ResumeRebalancing, HaltAll, CloseDriftHedge)"
  - "15 unit tests covering all branches and edge cases"
  - "strategy::risk_monitor module re-exported from strategy::mod"

affects:
  - 06-02 (DB persistence layer reads/writes RiskState)
  - 06-03 (watch-loop wiring calls evaluate() each tick)

tech-stack:
  added: []
  patterns:
    - "Pure state machine pattern: evaluate() is synchronous and infallible, takes external inputs (PnlSnapshot + Option<f64>) — no RPC or DB inside"
    - "High-water mark pattern for peak_pnl using simple > comparison"
    - "Module-level #![allow(dead_code)] to suppress lints until downstream plans wire the types"

key-files:
  created:
    - src/strategy/risk_monitor.rs
  modified:
    - src/strategy/mod.rs

key-decisions:
  - "evaluate() is synchronous and returns RiskAction directly (not Result<RiskAction>) — infallible by design"
  - "drift_margin_ratio passed as Option<f64> parameter so caller fetches it; keeps evaluate() testable without RPC mocking"
  - "No hysteresis on IL threshold: pause and resume use the same threshold (D-09)"
  - "Drawdown check skipped when peak_pnl <= 0.0 to avoid false triggers before any profit is realized"
  - "position_value == 0.0 yields il_pct = 0.0 (guard against division by zero)"

patterns-established:
  - "Pure-state risk evaluator pattern: all side effects (DB writes, RPC calls) deferred to caller"
  - "Evaluation order: halt_flag gate -> high-water mark -> drawdown -> IL -> Drift margin -> Continue"

requirements-completed: [RISK-01, RISK-02, RISK-03]

duration: 7min
completed: 2026-04-10
---

# Phase 6 Plan 01: RiskMonitor Core Module Summary

**Pure-state RiskMonitor with three-limit evaluate() — drawdown halts all, IL pauses/auto-resumes rebalancing, Drift margin triggers hedge-only close — verified by 15 unit tests covering all branches and edge cases.**

## Performance

- **Duration:** 7 min
- **Started:** 2026-04-10T13:49:13Z
- **Completed:** 2026-04-10T13:56:30Z
- **Tasks:** 1 of 1
- **Files modified:** 2

## Accomplishments

- Implemented `src/strategy/risk_monitor.rs` with `RiskMonitor`, `RiskState`, and `RiskAction` types matching the plan spec exactly
- `evaluate()` enforces correct D-05/D-06 evaluation order: halt_flag gate -> peak update -> drawdown -> IL -> Drift margin -> Continue
- 15 unit tests pass covering: halt_flag short-circuit, drawdown breach/no-breach/zero-peak, high-water mark monotonicity, IL pause/resume/propagation/no-hysteresis, zero position_value guard, evaluation order (drawdown fires before IL), all-limits-disabled, Drift margin above/below/disabled
- Updated `src/strategy/mod.rs` to add `pub mod risk_monitor` and re-export `RiskMonitor`, `RiskState`, `RiskAction`
- Passes `cargo clippy -- -D warnings` and `cargo fmt --check` cleanly

## Task Commits

1. **Task 1: Implement RiskMonitor core module with evaluate() and unit tests** - `9406ccc` (feat)

## Files Created/Modified

- `src/strategy/risk_monitor.rs` — RiskMonitor struct, RiskState, RiskAction enum, evaluate() method, 15 unit tests
- `src/strategy/mod.rs` — Added `pub mod risk_monitor` and `pub use risk_monitor::{RiskAction, RiskMonitor, RiskState}`

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing] Module-level dead_code suppression**
- **Found during:** Task 1 (clippy run)
- **Issue:** All public types triggered dead_code lints since Plans 02/03 haven't wired them into the watch loop yet. Per-item `#[allow(dead_code)]` attributes on struct/enum/impl were insufficient — the `impl` methods still triggered.
- **Fix:** Added `#![allow(dead_code)]` at module level in `risk_monitor.rs` and `#[allow(unused_imports)]` on the re-export line in `mod.rs`.
- **Files modified:** `src/strategy/risk_monitor.rs`, `src/strategy/mod.rs`

## Known Stubs

None — module is self-contained with no data sources needed. Plans 02 and 03 will wire DB persistence and watch-loop integration.

## Threat Flags

None — no new network endpoints, auth paths, file access patterns, or schema changes introduced. The `evaluate()` method is pure in-process computation as designed.

## Self-Check: PASSED

- `src/strategy/risk_monitor.rs` — exists, contains `pub struct RiskMonitor`, `pub struct RiskState`, `pub enum RiskAction`, `pub fn evaluate`, `HaltAll { drawdown_pct: f64 }`, `PauseRebalancing { il_pct: f64 }`, `ResumeRebalancing { il_pct: f64 }`, `CloseDriftHedge { margin_ratio: f64 }`, `il_usd.abs()`, `peak_pnl > 0.0`, `#[cfg(test)]`
- `src/strategy/mod.rs` — contains `pub mod risk_monitor`, `pub use risk_monitor::`
- Commit `9406ccc` — verified in git log
- All 15 tests pass: `cargo test --lib strategy::risk_monitor` exits 0
- `cargo clippy -- -D warnings` exits 0 (no errors, one pre-existing solana-client deprecation warning from dependency)
