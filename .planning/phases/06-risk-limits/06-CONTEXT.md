# Phase 6: Risk Limits - Context

**Gathered:** 2026-04-10
**Status:** Ready for planning

<domain>
## Phase Boundary

Phase 6 adds a `strategy::risk_monitor` module that evaluates three independent limit types on every WebSocket tick. When a limit is breached, the appropriate action fires (pause rebalancing, close LP, or close Drift hedge). Risk state (peak P&L, current drawdown, pause flag, halt flag) is persisted to DB and survives process restart. The phase also adds read-only Drift account fetching to make RISK-03 functionally real (no CPI — just RPC deserialization to get the current margin ratio).

</domain>

<decisions>
## Implementation Decisions

### Drift margin ratio source (LOCKED)
- **D-01:** Phase 6 adds a real read-only RPC fetch of the Drift User account to get the actual margin ratio — NOT a stub.
- **D-02:** No Drift CPI is added in Phase 6. This is monitoring only (read + evaluate + log/act), not execution.
- **D-03:** Crate vs. manual borsh layout for Drift account deserialization = Claude's discretion (prefer official crate if it avoids brittle offsets).

### Risk check timing (LOCKED)
- **D-04:** Risk monitor runs on **every incoming WebSocket tick**.
- **D-05:** Evaluation order in the watch loop: `pnl_history write` → **risk check** → `should_rebalance()` → slippage → execute.
- **D-06:** A breach halts the rest of the tick cycle immediately (no rebalance evaluation, no slippage check, no transaction attempt on a breached position).

### IL pause / auto-resume (LOCKED)
- **D-07:** IL pause is fully automatic. When instantaneous IL (from latest `pnl_history.il_usd` as a percentage of `position_value`) exceeds `--max-il`, rebalancing is paused. The `pause_flag` is set in the DB risk_state row.
- **D-08:** IL auto-resumes: when IL drops back below `--max-il` on any subsequent tick, `pause_flag` is cleared automatically — no operator action required.
- **D-09:** No hysteresis. Resume threshold = pause threshold = `--max-il`. If IL oscillates, the flag toggles; that is correct behavior.

### Drawdown close-all scope (LOCKED)
- **D-10:** Drawdown breach closes the LP position only via `OrcaExecutor` (`close_position` + `collect_fees`). Drift hedge close is NOT attempted; instead emit `tracing::error!` at CRITICAL level: `"halt: drawdown limit hit — Drift hedge close deferred (LIVE-02)"`.
- **D-11:** After the LP close, the process does NOT exit. Instead it sets `halt_flag = true` in the DB risk_state row and continues running (ticks keep being received and logged; rebalancing is permanently suppressed).
- **D-12:** The `halt_flag` survives process restart (it is in the DB). Operator must manually clear it (SQL UPDATE or a future CLI flag) before rebalancing can resume. A restart does not auto-clear the halt.

### Claude's Discretion
- Exact Rust struct layout for `RiskState` (the in-memory representation evaluated each tick)
- DB schema columns for `risk_state` table — must include at minimum: `pool_address`, `peak_pnl`, `current_drawdown_pct`, `pause_flag`, `halt_flag`, `updated_at`; additional columns at Claude's discretion
- Crate choice for Drift account deserialization (prefer official `drift-cpi` or `drift-client` crate; fall back to borsh layout only if crate adds too many transitive deps)
- Tracing span structure for risk breaches
- Where to add `--max-drawdown`, `--max-il`, `--drift-min-margin-ratio` CLI args (add to `Commands::Watch` variant alongside existing `--max-slippage-bps`)

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Existing watch loop integration points
- `src/main.rs` lines 463–900 — `Commands::Watch` arm; risk check inserts after `spawn_pnl_write` call (line ~1014) and before `should_rebalance()` call (line ~679)
- `src/main.rs` — `RunMode` enum and `ShadowGuard` match — risk monitor must check `halt_flag` and `pause_flag` before entering the rebalance path

### Strategy layer (add risk_monitor here)
- `src/strategy/mod.rs` — add `pub mod risk_monitor` and re-export `RiskMonitor`, `RiskState`, `RiskAction` here
- `src/strategy/signal.rs` — `should_rebalance()` signature; risk monitor sits before this call

### Execution layer (actions call these)
- `src/execution/orca_executor.rs` — `OrcaExecutor::execute_close_position()`, `execute_collect_fees()` — used by `close_all` action
- `src/execution/hedge.rs` — `log_hedge_stub()` — called alongside CRITICAL log when drawdown fires (to note Drift hedge close was skipped)

### Storage layer (extend this)
- `src/storage/writer.rs` — `spawn_pnl_write` writes `pnl_history` rows; risk monitor reads back latest row for IL and net_pnl values
- `src/storage/schema.sql` — add `risk_state` table here
- `src/storage/mod.rs` — `run_migrations()` entry point

### Requirements
- `.planning/REQUIREMENTS.md` — RISK-01 through RISK-04 definitions

### Prior phase context
- `.planning/phases/02-shadow-mode/02-CONTEXT.md` — shadow_rebalances table schema (Phase 6 does not modify this table)
- `.planning/phases/05-live-execution/05-CONTEXT.md` — OrcaExecutor wiring decisions, Drift defer rationale

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `execution::OrcaExecutor` — already has `execute_close_position()` and `execute_collect_fees()`; the `close_all` drawdown action reuses these directly
- `execution::log_hedge_stub()` — existing stub to emit Drift-deferred log; reuse for the drawdown hedge-close-skipped log
- `storage::writer::spawn_pnl_write` — already writes `net_pnl` and `position_value` to `pnl_history`; risk monitor reads these back to compute drawdown and IL percentage
- `strategy::signal::should_rebalance()` — risk monitor sits before this call; if `halt_flag` or `pause_flag` is set, skip this call entirely

### Established Patterns
- Fire-and-forget async writes via `tokio::spawn` (from Phase 1) — risk state DB writes follow the same pattern
- `ShadowGuard` enum with `Shadow`/`Live` variants for execution gating — risk monitor adds an analogous gate: if `halt_flag` is true, skip the rebalance path regardless of `ShadowGuard` state
- `anyhow::Result` for all error handling — risk monitor follows the same convention
- CLI args on `Commands::Watch` variant (`--max-slippage-bps` already there) — add three new flags to the same variant

### Integration Points
- `src/main.rs Watch` arm — after `spawn_pnl_write`, before `should_rebalance()`, insert: `risk_monitor.evaluate(&latest_pnl).await`
- `src/storage/schema.sql` — one new table: `risk_state`
- `src/strategy/mod.rs` — add `pub mod risk_monitor`

</code_context>

<specifics>
## Specific Implementation Notes

- IL percentage = `|il_usd| / position_value` from the latest `pnl_history` row for this pool_address
- Drawdown percentage = `(peak_pnl - current_pnl) / peak_pnl` where `peak_pnl` is the highest `net_pnl` value ever seen (stored in risk_state)
- If `--drift-min-margin-ratio` is set and Drift account fetch fails (e.g. RPC error), treat as "margin ratio OK" and log a warning — don't halt on a monitoring RPC failure
- The `risk_state` table is keyed on `pool_address` (consistent with shadow_rebalances and pnl_history). For v1 single-position focus, this is sufficient
- Risk state is loaded at watch startup. If no row exists for the pool, insert a fresh row with `peak_pnl = 0`, `pause_flag = false`, `halt_flag = false`
- On startup, if `halt_flag = true` is found: log a CRITICAL-level warning "halt flag set from previous session — rebalancing will remain halted until DB is manually cleared" and continue running (do not exit)

</specifics>

<deferred>
## Deferred Ideas

- Drift hedge close execution (RISK-03 action) — requires LIVE-02 Drift CPI, deferred to a future phase
- LIVE-04 atomicity between LP close and Drift hedge close — deferred with LIVE-02
- Telegram `/resume` command to clear halt_flag via bot (Phase 7 scope)
- CLI `watch --clear-halt` flag to programmatically clear halt without SQL — potential Phase 7 addition

</deferred>

---

*Phase: 06-risk-limits*
*Context gathered: 2026-04-10 via /gsd-discuss-phase 6*
