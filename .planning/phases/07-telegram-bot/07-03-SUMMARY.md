---
phase: 07-telegram-bot
plan: "03"
subsystem: bot
tags: [telegram, operator-control, queries, risk-monitor]
dependency_graph:
  requires: ["07-01", "07-02"]
  provides: ["TG-03", "TG-04", "TG-05"]
  affects: ["src/bot", "src/strategy/risk_monitor", "src/main"]
tech_stack:
  added: []
  patterns:
    - "DB query module (src/bot/queries.rs) decouples SQL from handler logic"
    - "In-memory + DB dual-write for operator_pause ensures immediate watch-loop response"
    - "D-04 separation: operator_pause cleared only by /resume, never by IL recovery"
key_files:
  created:
    - src/bot/queries.rs
  modified:
    - src/bot/commands.rs
    - src/bot/mod.rs
    - src/strategy/risk_monitor.rs
    - src/main.rs
decisions:
  - "operator_pause is persisted to DB AND updated in-memory immediately so the watch loop respects it within the same tick cycle"
  - "/resume only clears operator_pause, never pause_flag — per D-04 independence invariant"
  - "operator_pause gate placed AFTER risk match block so IL/drawdown halts take precedence"
metrics:
  duration: "~25 minutes"
  completed: "2026-04-10"
  tasks_completed: 2
  files_changed: 5
---

# Phase 7 Plan 03: Operator Commands (/status, /pause, /resume, /report) Summary

**One-liner:** Four operator Telegram commands fully wired to DB — /status shows live position + risk metrics, /pause and /resume control rebalancing independently of IL-triggered pause_flag (D-04), /report returns trailing 24h P&L aggregates.

## What Was Built

### src/bot/queries.rs (new)
DB query module with three async functions:
- `query_status`: fetches latest `pnl_history` row + `risk_state` row for the `/status` command
- `query_24h_report`: aggregates `SUM(fees_earned)`, `SUM(il_usd)`, `SUM(net_pnl)`, `COUNT(*)`, `MIN/MAX(price)` over trailing 24 hours from `pnl_history`
- `set_operator_pause`: `UPDATE risk_state SET operator_pause = $1` — used by both `/pause` and `/resume`

### src/bot/commands.rs (updated)
Replaced four stub handlers with real implementations:
- **handle_status**: calls `query_status`, formats multi-line message with position value, price, fees, IL, net P&L, drawdown %, peak P&L, and a human-readable status string (ACTIVE / PAUSED (operator) / PAUSED (IL limit) / HALTED)
- **handle_pause**: calls `set_operator_pause(true)`, immediately updates in-memory `RiskState.operator_pause = true`, sends acknowledgement
- **handle_resume**: calls `set_operator_pause(false)`, clears in-memory flag only (not `pause_flag` per D-04), appends IL-pause note if `pause_flag` is still active
- **handle_report**: calls `query_24h_report`, returns fees/IL/net P&L/price range; handles zero-row case gracefully

### src/strategy/risk_monitor.rs (updated)
- Added `pub operator_pause: bool` to `RiskState`
- `load_or_init` SELECT query and row mapping include `operator_pause`
- `load_or_init` INSERT default sets `operator_pause = FALSE`
- `persist_state` UPSERT includes `operator_pause` column and bind
- Test helper `make_state` updated with `operator_pause: false`

### src/main.rs (updated)
- Startup log: warns if `risk_state.operator_pause` is true from previous session
- Watch loop: operator_pause gate added after risk match block, before `should_rebalance` — skips rebalance tick when `rm.state.operator_pause` is true

## Deviations from Plan

None — plan executed exactly as written.

## Threat Surface Scan

No new network endpoints, auth paths, or schema changes beyond what the plan's threat model covers. All `pool_address` parameters are bound via `$1`/`$2` parameterized queries (T-07-08 mitigated). All handlers gated by `chat_id` check (T-07-07 mitigated).

## Self-Check: PASSED

Files created/exist:
- src/bot/queries.rs — FOUND
- src/bot/commands.rs — FOUND (modified)
- src/strategy/risk_monitor.rs — FOUND (modified)
- src/main.rs — FOUND (modified)

Commits:
- 914c824 feat(07-03): add queries module and operator_pause to RiskState
- 11a5587 feat(07-03): implement /status, /pause, /resume, /report and wire operator_pause gate
