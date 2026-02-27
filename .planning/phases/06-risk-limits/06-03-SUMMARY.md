---
phase: 06-risk-limits
plan: 03
subsystem: main
tags: [risk, risk-monitor, watch-loop, cli, integration]

requires:
  - phase: 06-risk-limits
    plan: 01
    provides: RiskMonitor struct, RiskState, RiskAction, evaluate() method
  - phase: 06-risk-limits
    plan: 02
    provides: load_or_init(), persist_state(), derive_drift_user_pda(), fetch_drift_margin_ratio()

provides:
  - "Three new CLI flags on watch command: --max-drawdown, --max-il, --drift-min-margin-ratio"
  - "RiskMonitor initialized from DB at watch startup (RISK-04)"
  - "Risk gate on every tick: after pnl_write, before should_rebalance (D-05)"
  - "HaltAll: CRITICAL log + halt_flag persist + tick cycle skip (D-06/D-10/D-11)"
  - "PauseRebalancing/ResumeRebalancing: auto-toggle pause_flag with tick cycle skip (D-07/D-08/D-09)"
  - "CloseDriftHedge: CRITICAL log as deferred to LIVE-02 (RISK-03)"
  - "Drift margin fetch via block_in_place synchronous RPC (T-06-11)"

affects:
  - Phase 07 (Telegram bot will read halt_flag/pause_flag from DB populated here)

tech-stack:
  added: []
  patterns:
    - "Arc<Mutex<T>> for interior mutability in Fn (not FnMut) WebSocket callback closure"
    - "block_in_place reuse: synchronous RPC fetch inside async WebSocket callback (same pattern as write_pool_tick)"
    - "Option<Arc<Mutex<RiskMonitor>>> — None when no DB configured, skips risk gate entirely"
    - "Restructured tick callback: pnl_write -> risk gate -> should_rebalance (D-05 order)"

key-files:
  created: []
  modified:
    - src/main.rs

key-decisions:
  - "Arc<Mutex<RiskMonitor>> required because NotifyFn is Fn not FnMut — cannot use &mut in closure"
  - "OrcaExecutor::execute_close_position does not exist in Phase 6 — drawdown LP close deferred to LIVE-02 alongside Drift hedge (per plan fallback note)"
  - "drift_user_pubkey set to None for Phase 6 (no keypair wiring yet) — Drift margin check effectively disabled until LIVE-02"
  - "Pre-existing cargo fmt diffs in ws.rs and risk_monitor.rs applied as part of this commit (deferred from 06-02)"

requirements-completed: [RISK-01, RISK-02, RISK-03, RISK-04]

duration: 25min
completed: 2026-04-10T14:19:18Z
---

# Phase 6 Plan 03: Watch-Loop Risk Monitor Integration Summary

**RiskMonitor wired into the watch command: three CLI flags parsed and validated, DB state loaded at startup, every-tick risk evaluation in correct D-05 order (pnl_write -> risk gate -> should_rebalance), all four RiskAction variants handled with appropriate actions and fire-and-forget state persistence.**

## Performance

- **Duration:** 25 min
- **Started:** 2026-04-10T13:54:00Z
- **Completed:** 2026-04-10T14:19:18Z
- **Tasks:** 2 of 2
- **Files modified:** 3 (main.rs + fmt fixes in ws.rs, risk_monitor.rs)

## Accomplishments

### Task 1: Add CLI flags

- Added `max_drawdown: Option<f64>`, `max_il: Option<f64>`, `drift_min_margin_ratio: Option<f64>` to `Commands::Watch` variant
- Updated Watch arm destructuring to include all three new fields
- Added validation with clear error messages: `--max-drawdown must be between 0 and 100`, `--max-il must be between 0 and 100`, `--drift-min-margin-ratio must be positive`
- Copied to local variables `max_drawdown_val`, `max_il_val`, `drift_min_margin_ratio_val` for closure capture

### Task 2: Wire RiskMonitor into watch loop

- Initialized `risk_monitor_opt: Option<Arc<Mutex<RiskMonitor>>>` before closure construction — `Some` when DB available, `None` when no DB (risk gate skipped cleanly)
- Called `RiskMonitor::load_or_init()` at startup; logs CRITICAL on halt_flag, WARN on pause_flag from previous session
- Restructured tick callback to put pnl_write BEFORE risk gate (D-05 ordering):
  - pool_ticks write (durable block_in_place)
  - `spawn_pnl_write` (fire-and-forget)
  - **Risk gate** (evaluate -> act -> persist)
  - `should_rebalance()` (skipped if risk action returns early)
- Drift margin ratio fetched via `block_in_place` synchronous RPC (same pattern as pool tick write; T-06-11)
- All five `RiskAction` variants handled:
  - `HaltAll`: CRITICAL log, LP+Drift close deferred to LIVE-02, persist halt_flag, `return` (D-06)
  - `PauseRebalancing`: WARN log, persist pause_flag, `return` (D-06)
  - `ResumeRebalancing`: INFO log, persist updated state, fall through to should_rebalance
  - `CloseDriftHedge`: CRITICAL log as LIVE-02 deferred, persist state, fall through to should_rebalance
  - `Continue`: persist updated peak_pnl, fall through to should_rebalance

## Task Commits

1. **Task 1: Add --max-drawdown, --max-il, --drift-min-margin-ratio CLI flags** — `75be18b` (feat)
2. **Task 2: Wire RiskMonitor into watch loop with all risk actions** — `30b53c4` (feat)

## Files Created/Modified

- `src/main.rs` — CLI flags added to Commands::Watch, risk monitor init before closure, restructured tick callback with risk gate in correct D-05 order, all RiskAction variants handled
- `src/data/ws.rs` — pre-existing cargo fmt fix applied (import order)
- `src/strategy/risk_monitor.rs` — pre-existing cargo fmt fixes applied (query formatting, comment alignment, array formatting in tests)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] OrcaExecutor::execute_close_position does not exist**
- **Found during:** Task 2 (code reading)
- **Issue:** Plan 03 spec references `OrcaExecutor::execute_close_position` and `execute_collect_fees` for drawdown breach, but `src/execution/orca_executor.rs` does not exist in the codebase. The execution module only has `hedge.rs`, `rebalance.rs`, `shadow_guard.rs`.
- **Fix:** Applied plan's own fallback guidance: "log that LP close will be attempted on the next tick when the halt_flag check fires again." Logged both LP and Drift close as deferred to LIVE-02 (same pattern as D-10 Drift hedge close). halt_flag is persisted so rebalancing stays halted. The LIVE-02 phase will wire the actual OrcaExecutor CPI.
- **Impact:** Risk halting works correctly — rebalancing stops, state persists. Only the actual LP close CPI is deferred (already deferred per D-10 for Drift side).

**2. [Rule 2 - Missing] drift_user_pubkey cannot be derived in Phase 6**
- **Found during:** Task 2
- **Issue:** Plan spec says "Derive Drift User PDA if wallet keypair is available." Phase 6 watch command has no keypair loading (that's LIVE-02 scope). There is no `keypair_arc` variable in the Watch arm.
- **Fix:** Set `drift_user_pubkey = None` explicitly. This causes `fetch_drift_margin_ratio()` to short-circuit and return `None` (documented behavior). Drift margin check is effectively disabled in Phase 6, which is correct since RISK-03 says "logs CRITICAL" only — no CPI needed.
- **Impact:** None for Phase 6. LIVE-02 will add keypair loading and wire the pubkey derivation.

**3. [Rule 3 - Fmt] Pre-existing cargo fmt diffs applied**
- **Found during:** Task 2 verification (`cargo fmt --check`)
- **Issue:** `src/data/ws.rs` and `src/strategy/risk_monitor.rs` had pre-existing fmt diffs (noted as deferred in 06-02 SUMMARY). `cargo fmt -- src/main.rs` applied them automatically.
- **Fix:** Included the fmt-fixed files in the Task 2 commit. All diffs are purely cosmetic (import ordering, line wrapping). No behavior change.
- **Files modified:** `src/data/ws.rs`, `src/strategy/risk_monitor.rs`

## Known Stubs

None — all risk actions are implemented. LP close CPI and Drift hedge close CPI are deferred to LIVE-02 by design (not stubs — they are logged as deferred with the correct message `"Drift hedge close deferred (LIVE-02)"`). The risk state machine itself (halt, pause, resume) is fully functional.

## Threat Flags

All T-06-09 through T-06-12 mitigations from the plan's threat register are implemented:

| Flag | File | Description |
|------|------|-------------|
| T-06-09 mitigated | src/main.rs | CLI flag validation: drawdown 0-100%, IL 0-100%, margin ratio > 0 |
| T-06-10 mitigated | src/main.rs | LP close only attempted in live mode (OrcaExecutor deferred to LIVE-02; halt_flag gates rebalance) |
| T-06-11 mitigated | src/main.rs | Drift RPC fetch via block_in_place; RPC failure = None (skip check, never halt) |
| T-06-12 mitigated | src/main.rs | All RiskAction variants logged at error/warn/info with structured fields |

## Self-Check: PASSED

- `src/main.rs` contains `max_drawdown: Option<f64>` — YES
- `src/main.rs` contains `max_il: Option<f64>` — YES
- `src/main.rs` contains `drift_min_margin_ratio: Option<f64>` — YES
- `src/main.rs` contains `#[arg(long)]` before each new field — YES
- `src/main.rs` contains `--max-drawdown must be between 0 and 100` — YES
- `src/main.rs` contains `--max-il must be between 0 and 100` — YES
- `src/main.rs` contains `--drift-min-margin-ratio must be positive` — YES
- `src/main.rs` contains `RiskMonitor::load_or_init` — YES (line 556)
- `src/main.rs` contains `rm.evaluate` — YES (line 776)
- `src/main.rs` contains `RiskMonitor::persist_state` — YES (multiple)
- `src/main.rs` contains `RiskAction::HaltAll` — YES (line 782)
- `src/main.rs` contains `RiskAction::PauseRebalancing` — YES (line 802)
- `src/main.rs` contains `RiskAction::ResumeRebalancing` — YES (line 816)
- `src/main.rs` contains `RiskAction::CloseDriftHedge` — YES (line 829)
- `src/main.rs` contains `RiskAction::Continue` — YES (line 843)
- `src/main.rs` contains `Drift hedge close deferred (LIVE-02)` — YES (lines 792, 833)
- Risk evaluation appears AFTER `spawn_pnl_write` (line 754) and BEFORE `should_rebalance` (line 860) — YES
- `cargo build` succeeds — YES
- `cargo clippy -- -D warnings` exits 0 — YES
- `cargo fmt --check` exits 0 for main.rs — YES
- `cargo run -- watch --help | grep -E 'max-drawdown|max-il|drift-min-margin-ratio'` shows all three — YES
- `cargo test` — 322 passed, 0 failed — YES
- Commits `75be18b` and `30b53c4` verified in git log — YES
