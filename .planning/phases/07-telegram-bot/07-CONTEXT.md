---
phase: 07-telegram-bot
created: 2026-04-10
status: ready
---

# Phase 07 Context: Telegram Bot

## Domain Boundary

Operator notification and command interface layered onto the existing `watch` loop.
Scope: proposal flow (/approve), operator pause (/pause, /resume), status query (/status), 24h P&L report (/report).
Out of scope: new rebalance strategies, additional risk checks, multi-pool management.

## Decisions

### D-01: Bot process model — Integrated tokio task

The Telegram bot runs as a `tokio::spawn` task inside the same `watch` binary.
Bot and watch loop share in-process state via `Arc<Mutex<RiskMonitor>>` and tokio channels.
Single binary, single env file, no IPC overhead.

**Implication for planner:** No separate binary or crate needed. Add `teloxide` to existing `Cargo.toml`. Bot task spawned alongside the WebSocket loop at watch startup.

### D-02: /approve interaction model — tokio::oneshot channel

When a rebalance signal fires, the watch loop:
1. Sends a Telegram proposal message
2. Parks the rebalance execution on `tokio::time::timeout(approve_timeout, oneshot_rx.await)`
3. Ticks continue arriving and P&L writes continue during the wait window
4. On approval: execute rebalance
5. On timeout or explicit `/reject`: log skip to DB, continue

**Implication for planner:** A `PendingApproval` state needs to be tracked (e.g., `Arc<Mutex<Option<oneshot::Sender<bool>>>>`) — the bot handler sends `true` to this sender when `/approve` arrives. Only one approval window can be open at a time.

### D-03: Approval timeout — `--approve-timeout-secs`, default 300

`--approve-timeout-secs` flag on the `watch` subcommand, default 300 seconds (5 min).
Configurable per session. Timeout fires `skip + log` identical to explicit `/reject`.

**Implication for planner:** Add to the `Commands::Watch` arg struct alongside the existing risk limit flags.

### D-04: /pause semantics — separate `operator_pause` column

Telegram `/pause` uses a new `operator_pause` boolean column in the `risk_state` table,
independent from the IL-triggered `pause_flag`.

Watch loop skips rebalancing if `pause_flag OR operator_pause` is true.

`/resume` clears only `operator_pause` — it does NOT clear `pause_flag` (that is owned by the risk monitor's IL recovery logic).

**Rationale:** Prevents IL recovery events from overriding an explicit operator pause. The two pause reasons have different lifecycles.

**Implication for planner:**
- Schema: `ALTER TABLE risk_state ADD COLUMN operator_pause BOOLEAN NOT NULL DEFAULT FALSE`
- `RiskMonitor::load_or_init()` loads both flags
- Watch loop rebalance gate: `if risk_state.pause_flag || risk_state.operator_pause { skip }`
- Bot `/pause` handler: UPDATE risk_state SET operator_pause = true
- Bot `/resume` handler: UPDATE risk_state SET operator_pause = false

## Canonical Refs

- `.planning/REQUIREMENTS.md` — TG-01 through TG-05 acceptance criteria
- `.planning/ROADMAP.md` — Phase 7 success criteria (SC-1 through SC-5)
- `src/strategy/risk_monitor.rs` — RiskMonitor, RiskState, pause_flag, halt_flag patterns
- `src/storage/schema.sql` — risk_state table structure to extend
- `src/main.rs` — Commands::Watch arg struct, watch loop structure, risk gate pattern

## Deferred Ideas

None raised during discussion.
