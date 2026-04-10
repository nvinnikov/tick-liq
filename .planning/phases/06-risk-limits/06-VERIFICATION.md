---
phase: 06-risk-limits
verified: 2026-04-10T15:00:00Z
status: gaps_found
score: 10/12 must-haves verified
overrides_applied: 0
gaps:
  - truth: "When cumulative P&L drawdown exceeds --max-drawdown, LP position and hedge are closed and execution halts"
    status: partial
    reason: "Execution halt (halt_flag set + persisted, rebalancing stopped) is fully implemented. LP close CPI and Drift hedge close CPI are not executed — both are logged as 'deferred (LIVE-02)' because OrcaExecutor::execute_close_position does not exist in the codebase."
    artifacts:
      - path: "src/main.rs"
        issue: "HaltAll branch logs deferred action for LP close, does not call any executor"
      - path: "src/execution/orca_executor.rs"
        issue: "File does not exist — execute_close_position was never implemented"
    missing:
      - "OrcaExecutor::execute_close_position and execute_collect_fees must exist and be called on HaltAll breach"
  - truth: "When Drift margin ratio falls below --drift-min-margin-ratio, the Drift hedge is closed while LP remains open"
    status: partial
    reason: "CloseDriftHedge action is detected, logged at error level, and state is persisted. No actual Drift hedge close CPI is executed — it is logged as 'Drift hedge close deferred (LIVE-02)'. Additionally, drift_user_pubkey is always None in Phase 6 so fetch_drift_margin_ratio() never fetches from RPC — Drift margin monitoring is effectively disabled at runtime."
    artifacts:
      - path: "src/main.rs"
        issue: "CloseDriftHedge branch logs deferred action only; no CPI executed. drift_user_pubkey = None hardcoded (line 574) so Drift RPC fetch is always skipped."
    missing:
      - "Drift hedge close CPI execution on breach (LIVE-02 scope)"
      - "drift_user_pubkey must be derived from keypair when wallet is available"
deferred:
  - truth: "LP close CPI on drawdown breach and Drift hedge CPI on margin breach"
    addressed_in: "Phase 5 (LIVE-02)"
    evidence: "ROADMAP Phase 5 success criteria: 'Drift Protocol perp hedge is updated in the same rebalance cycle'. Plan 06-03 key-decisions: 'OrcaExecutor::execute_close_position does not exist in Phase 6 — drawdown LP close deferred to LIVE-02 alongside Drift hedge (per plan fallback note)'"
---

# Phase 6: Risk Limits Verification Report

**Phase Goal:** The running system enforces configurable hard limits on drawdown, instantaneous IL, and Drift margin ratio, taking the correct per-limit action automatically and surviving process restarts.
**Verified:** 2026-04-10T15:00:00Z
**Status:** gaps_found
**Re-verification:** No — initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | When cumulative P&L drawdown exceeds --max-drawdown, LP position and hedge are closed and execution halts | PARTIAL | Halt (halt_flag + rebalancing stopped) works. LP close CPI not executed; OrcaExecutor::execute_close_position does not exist. Deferred to LIVE-02. |
| 2 | When instantaneous IL exceeds --max-il, rebalancing is paused but position stays open; resumes when IL drops | VERIFIED | PauseRebalancing/ResumeRebalancing correctly implemented in evaluate() with no hysteresis (D-09). Wired in main.rs lines 802-828 to skip should_rebalance on pause, fall through on resume. pause_flag persisted to DB. |
| 3 | When Drift margin ratio falls below --drift-min-margin-ratio, the Drift hedge is closed while LP remains open | PARTIAL | CloseDriftHedge logged at error level, LP rebalance falls through. No hedge close CPI executed. Also: drift_user_pubkey is hardcoded None in Phase 6 so Drift RPC fetch never runs. |
| 4 | Risk state (peak_value, current_drawdown) is persisted to DB and limits re-evaluated correctly after process restart | VERIFIED | risk_state table exists in schema.sql with all required columns. load_or_init uses SELECT-then-INSERT preserving halt_flag (D-12). persist_state is fire-and-forget tokio::spawn. Called after every evaluate() for all five RiskAction variants. |
| 5 | RiskMonitor::evaluate() returns correct RiskAction for each threshold breach | VERIFIED | 18 unit tests pass (19 registered, 1 ignored as live RPC test). All branches covered including zero-peak guard, zero-position-value guard, evaluation order, halt_flag short-circuit, IL oscillation, no-hysteresis. |
| 6 | Drawdown check skipped when peak_pnl <= 0 | VERIFIED | Code: `if self.state.peak_pnl > 0.0` guard in evaluate(). Test: drawdown_skipped_when_peak_not_positive passes. |
| 7 | IL percentage uses abs(il_usd) / position_value | VERIFIED | Code line 381: `snap.il_usd.abs() / snap.position_value * 100.0`. Confirmed in source. |
| 8 | Evaluation order: halt_flag -> drawdown -> IL -> Drift margin -> Continue | VERIFIED | Source order in evaluate(): halt_flag gate (line 352), peak update (360), drawdown (365), IL (380), Drift margin (406), Continue (418). Test drawdown_fires_before_il_check passes. |
| 9 | Drift margin check returns Continue when drift_min_margin_ratio is None | VERIFIED | Pattern `if let Some(min_ratio) = self.drift_min_margin_ratio` at line 406 — skips check when None. Test drift_min_margin_none_returns_continue passes. |
| 10 | risk_state table exists with pool_address PRIMARY KEY and all required columns | VERIFIED | schema.sql lines 67-74: CREATE TABLE IF NOT EXISTS risk_state with pool_address TEXT PRIMARY KEY, peak_pnl, current_drawdown_pct, pause_flag, halt_flag, updated_at. |
| 11 | RiskState loaded from DB at startup; missing row creates fresh state | VERIFIED | load_or_init(): SELECT-then-INSERT (not upsert). Existing row returned as-is; missing row gets INSERT with ON CONFLICT DO NOTHING then fresh default RiskState returned. halt_flag=true emits error log. |
| 12 | CLI accepts --max-drawdown, --max-il, --drift-min-margin-ratio flags | VERIFIED | All three present in Commands::Watch at lines 75-83 of main.rs. Validation at lines 462-476. `cargo run -- watch --help` shows all three flags. |

**Score:** 10/12 truths verified (2 partial — gaps_found)

### Deferred Items

Items not yet met but explicitly addressed in later milestone phases.

| # | Item | Addressed In | Evidence |
|---|------|-------------|----------|
| 1 | LP close CPI on drawdown breach | Phase 5 (LIVE-02) | ROADMAP Phase 5 LIVE-02: "Drift Protocol perp hedge is updated in the same rebalance cycle". Plan 06-03 key-decision: "OrcaExecutor::execute_close_position does not exist in Phase 6 — drawdown LP close deferred to LIVE-02" |
| 2 | Drift hedge close CPI on margin breach | Phase 5 (LIVE-02) | Same as above — both CPI executions are co-located in LIVE-02 scope |

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `src/strategy/risk_monitor.rs` | RiskMonitor struct, RiskState, RiskAction, evaluate() | VERIFIED | All types present. 18 tests pass. Compiles clean. |
| `src/strategy/mod.rs` | pub mod risk_monitor re-export | VERIFIED | Lines 1 and 6: pub mod risk_monitor + pub use risk_monitor::{RiskAction, RiskMonitor, RiskState} |
| `src/storage/schema.sql` | risk_state table DDL | VERIFIED | Lines 67-74 present with all required columns and PRIMARY KEY |
| `src/main.rs` | CLI flags, risk monitor init, tick-level evaluation wiring | VERIFIED (partial wiring) | Flags and init fully wired. All 5 RiskAction variants handled. LP close and Drift close CPIs not executed — logged as deferred. drift_user_pubkey hardcoded None. |

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|-----|--------|---------|
| src/strategy/risk_monitor.rs | src/storage/writer.rs | PnlSnapshot as evaluate() input | VERIFIED | `use crate::storage::writer::PnlSnapshot` at line 13. make_snap() in tests constructs PnlSnapshot directly. |
| src/strategy/risk_monitor.rs | src/storage/schema.sql | sqlx queries matching risk_state columns | VERIFIED | load_or_init() SELECTs peak_pnl, current_drawdown_pct, pause_flag, halt_flag, updated_at. persist_state() INSERTs all columns matching DDL. |
| src/strategy/risk_monitor.rs | solana_client::rpc_client | get_account_data for Drift User PDA | VERIFIED (disabled at runtime) | fetch_drift_margin_ratio() calls rpc.get_account_data(&pubkey). BUT drift_user_pubkey is None in Phase 6 so this code path is never reached. |
| src/main.rs | src/strategy/risk_monitor.rs | RiskMonitor::load_or_init + evaluate + persist_state | VERIFIED | load_or_init at line 556, rm.evaluate at line 776, RiskMonitor::persist_state at lines 795/809/823/836/846. |
| src/main.rs | src/execution/orca_executor.rs | execute_close_position + execute_collect_fees on HaltAll | NOT WIRED | orca_executor.rs does not exist. LP close on drawdown is deferred to LIVE-02. |

### Data-Flow Trace (Level 4)

| Artifact | Data Variable | Source | Produces Real Data | Status |
|----------|---------------|--------|--------------------|--------|
| src/main.rs (risk gate) | snap (PnlSnapshot) | Computed from on-chain tick data: computed_fees_earned, computed_il_usd, computed_position_value | Yes — derived from live WebSocket events and RPC data | FLOWING |
| src/strategy/risk_monitor.rs (evaluate) | snap.net_pnl, snap.il_usd, snap.position_value | PnlSnapshot from watch loop tick callback | Yes — real on-chain data when watch command runs | FLOWING |
| src/strategy/risk_monitor.rs (fetch_drift_margin_ratio) | drift_margin_ratio | Drift User PDA RPC account fetch | STATIC/DISCONNECTED — drift_user_pubkey is always None in Phase 6; RPC fetch never executes | DISCONNECTED |

### Behavioral Spot-Checks

| Behavior | Command | Result | Status |
|----------|---------|--------|--------|
| CLI flags accepted by watch help | `/Users/n.vinnikov/.cargo/bin/cargo run -- watch --help \| grep -E "max-drawdown\|max-il\|drift-min-margin"` | All three flags shown: --max-drawdown, --max-il, --drift-min-margin-ratio | PASS |
| All risk_monitor unit tests pass | `/Users/n.vinnikov/.cargo/bin/cargo test --lib strategy::risk_monitor` | 18 passed; 0 failed; 1 ignored (live RPC) | PASS |
| Project compiles | `/Users/n.vinnikov/.cargo/bin/cargo build` | Finished dev profile. One pre-existing solana-client future-incompat warning (not from project code). | PASS |
| Evaluation ordering correct | Code review: line 754 spawn_pnl_write, line 760 risk gate, line 860 should_rebalance | spawn_pnl_write (754) -> risk gate (760) -> should_rebalance (860) | PASS |
| halt_flag preserved on restart | load_or_init() SELECT-then-INSERT pattern; INSERT ON CONFLICT DO NOTHING | Correct — existing halt_flag never overwritten; tracing::error! emitted when halt_flag=true | PASS |

### Requirements Coverage

| Requirement | Source Plan | Description | Status | Evidence |
|-------------|------------|-------------|--------|----------|
| RISK-01 | 06-01, 06-03 | --max-drawdown: drawdown breach closes LP and hedge, halts execution | PARTIAL | Halt works (halt_flag set, rebalancing stops). LP close CPI missing (orca_executor.rs absent). Logged as deferred to LIVE-02. |
| RISK-02 | 06-01, 06-03 | --max-il: IL breach pauses rebalancing; position stays open; auto-resumes | SATISFIED | PauseRebalancing/ResumeRebalancing fully implemented, wired, and tested. pause_flag persisted. |
| RISK-03 | 06-01, 06-02, 06-03 | --drift-min-margin-ratio: Drift hedge closed when below threshold, LP stays open | PARTIAL | CloseDriftHedge detected and logged. No CPI executed. drift_user_pubkey always None so Drift margin fetch never runs in practice. |
| RISK-04 | 06-02, 06-03 | Risk state persisted in DB; survives process restart | SATISFIED | risk_state table with correct schema. load_or_init preserves halt_flag (D-12). persist_state fire-and-forget after every evaluate(). |

### Anti-Patterns Found

| File | Line | Pattern | Severity | Impact |
|------|------|---------|----------|--------|
| src/main.rs | 574 | `let drift_user_pubkey: Option<solana_sdk::pubkey::Pubkey> = None;` hardcoded | Warning | Drift margin monitoring is completely disabled at runtime — fetch_drift_margin_ratio() short-circuits immediately on None pubkey. The CLI flag is accepted and validated but has no operational effect in Phase 6. |
| src/main.rs | 788-792 | LP close CPI replaced by log message "deferred (LIVE-02)" | Warning (intentional deferral) | Drawdown halt does halt rebalancing correctly via halt_flag. Only the actual LP close transaction is missing. This is a documented intentional deferral, not an accidental stub. |

### Human Verification Required

None — all observable behaviors verified programmatically.

### Gaps Summary

Two gaps blocking full ROADMAP success criteria fulfillment:

**Gap 1 — LP close CPI missing (RISK-01 partial):** The drawdown halt mechanism correctly sets halt_flag and stops all rebalancing. However, the ROADMAP SC-1 says "LP position and hedge are closed" — neither close is executed. `src/execution/orca_executor.rs` does not exist (OrcaExecutor was planned in Phase 5 but execute_close_position was not implemented there either). Both LP close and Drift hedge close on drawdown are logged as deferred to LIVE-02.

**Gap 2 — Drift margin monitoring disabled at runtime (RISK-03 partial):** The `fetch_drift_margin_ratio()` method is implemented with real RPC fetch logic. However, `drift_user_pubkey` is always set to `None` in the watch loop initialization (main.rs line 574) because keypair loading is not available in Phase 6 shadow mode. As a result, the Drift margin check never fires in practice. The CloseDriftHedge action can only be tested by constructing a RiskMonitor with a pubkey manually (unit tests do this). Additionally, even if monitoring worked, the CloseDriftHedge handler only logs a deferred message — no CPI is executed.

**Why these are gaps and not deferred:** The ROADMAP explicitly states that the closing of LP and hedge positions is part of Phase 6 success criteria. While the SUMMARY documents these as deferred to LIVE-02, LIVE-02 is scoped under Phase 5 requirements (already marked pending in REQUIREMENTS.md). The deferred work has no clear home in a future phase that hasn't been planned yet. The risk state machine itself (halt, pause, resume) is complete and production-quality; only the execution-side effects are missing.

---

_Verified: 2026-04-10T15:00:00Z_
_Verifier: Claude (gsd-verifier)_
