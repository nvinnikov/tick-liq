---
phase: 07-telegram-bot
plan: "01"
subsystem: bot
tags: [teloxide, telegram, tokio, rust, bot, commands]

# Dependency graph
requires:
  - phase: 06-risk-limits
    provides: RiskMonitor, RiskState, risk_state DB table
provides:
  - teloxide bot module with BotState and spawn_bot() entry point
  - Command enum with Status, Pause, Resume, Report, Approve handler stubs
  - operator_pause column in risk_state table (D-04)
  - --telegram and --approve-timeout-secs CLI flags on watch command
  - pending_approval Arc<Mutex<Option<Sender<bool>>>> channel for Plan 02
affects: [07-02, 07-03]

# Tech tracking
tech-stack:
  added: [teloxide 0.13, dptree 0.3]
  patterns:
    - "Bot module as tokio::spawn task sharing Arc<Mutex<RiskMonitor>> with watch loop (D-01)"
    - "BotState holds db_pool, risk_monitor, pending_approval for handler injection via dptree"
    - "Dual-crate-root module declaration: pub mod bot in lib.rs + mod bot in main.rs"

key-files:
  created:
    - src/bot/mod.rs
    - src/bot/commands.rs
  modified:
    - Cargo.toml
    - src/lib.rs
    - src/main.rs
    - src/storage/schema.sql

key-decisions:
  - "BotState fields annotated #[allow(dead_code)] — used by Plans 02/03, not Plan 01 stubs"
  - "operator_pause column added as ALTER TABLE IF NOT EXISTS in schema.sql (idempotent)"
  - "pending_approval channel created regardless of --telegram flag so Plans 02/03 can wire it unconditionally"

patterns-established:
  - "Bot handler stubs: all use _state parameter, reply with 'not yet implemented (Plan NN)' placeholder"
  - "spawn_bot() returns JoinHandle<()> for crash detection by watch loop"
  - "TELEGRAM_BOT_TOKEN loaded exclusively via std::env::var — never CLI/config (T-07-01 mitigation)"

requirements-completed: [TG-01, TG-04]

# Metrics
duration: 30min
completed: 2026-04-10
---

# Phase 07 Plan 01: Telegram Bot Infrastructure Summary

**teloxide 0.13 bot module with 5 command stubs wired into watch as tokio task, operator_pause DB column, and pending_approval channel for rebalance approval flow**

## Performance

- **Duration:** ~30 min
- **Started:** 2026-04-10T17:00:00Z
- **Completed:** 2026-04-10T17:27:07Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments

- Created `src/bot/` module with `BotState` struct (db_pool, risk_monitor, pending_approval) and `spawn_bot()` tokio task entry point
- Defined `Command` enum with all 5 commands (Status, Pause, Resume, Report, Approve) routed via teloxide dispatcher + dptree handler chain
- Added `operator_pause BOOLEAN NOT NULL DEFAULT FALSE` column to `risk_state` table (D-04: separate from IL-triggered pause_flag)
- Wired `--telegram` and `--approve-timeout-secs` flags to `Commands::Watch`; bot spawned at watch startup with shared `Arc<Mutex<RiskMonitor>>` and `pending_approval` oneshot channel

## Task Commits

Each task was committed atomically:

1. **Task 1: Add teloxide dependency, create bot module with command router and schema migration** - `b78976c` (feat)
2. **Task 2: Wire bot spawn into watch command with CLI flag and shared state** - `9868e13` (feat)

## Files Created/Modified

- `src/bot/mod.rs` - BotState struct and spawn_bot() entry point; reads TELEGRAM_BOT_TOKEN from env only
- `src/bot/commands.rs` - Command enum (Status/Pause/Resume/Report/Approve), build_handler(), 5 stub handlers
- `Cargo.toml` - Added teloxide 0.13 + dptree 0.3 dependencies
- `src/lib.rs` - Added `pub mod bot;` (lib crate root declaration)
- `src/main.rs` - Added `mod bot;` (binary crate root), --telegram/--approve-timeout-secs flags, bot spawn wiring
- `src/storage/schema.sql` - Added `ALTER TABLE risk_state ADD COLUMN IF NOT EXISTS operator_pause`

## Decisions Made

- `BotState` fields annotated with `#[allow(dead_code)]` — all 5 fields are consumed by Plan 02/03 implementations; Plan 01 stubs use `_state` parameter to keep clippy clean
- `pending_approval` channel allocated unconditionally (regardless of `--telegram`) so Plans 02/03 can wire it into the tick callback without a flag check
- Schema `ALTER TABLE` statement placed in schema.sql as a separate idempotent statement (not inside CREATE TABLE) so it can be applied to existing deployments

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] Added `#[allow(dead_code)]` to BotState**
- **Found during:** Task 2 (clippy verification)
- **Issue:** Handler stubs use `_state` parameter; `cargo clippy -- -D warnings` emitted error for unread struct fields
- **Fix:** Added `#[allow(dead_code)]` attribute to `BotState` struct with explanatory comment
- **Files modified:** src/bot/mod.rs
- **Verification:** `cargo clippy -- -D warnings` exits 0
- **Committed in:** `9868e13` (Task 2 commit, updated mod.rs)

---

**Total deviations:** 1 auto-fixed (1 missing critical / clippy compliance)
**Impact on plan:** Necessary for clippy -D warnings compliance. No scope creep.

## Issues Encountered

- Worktree was initialized from an older base commit (7a4f7c1, phase 03 state) rather than the target 6d01680 (phase 07 planning). Fixed with `git reset --soft 6d01680` followed by `git checkout HEAD -- .` to restore phase 06 working tree state including `src/strategy/risk_monitor.rs`.

## Threat Surface Scan

No new network endpoints, auth paths, or trust-boundary schema changes beyond what the plan's threat model covers.

- T-07-01 mitigated: TELEGRAM_BOT_TOKEN loaded exclusively via `std::env::var` in `spawn_bot()` — never from CLI args or config files
- T-07-02 residual: handler stubs do not execute real actions; chat_id allowlist deferred to Plan 02 as planned
- T-07-03 mitigated: Plan 01 has no DB writes in handlers; parameterized queries enforced in Plans 02/03

## Next Phase Readiness

- Bot infrastructure complete; Plans 02 and 03 can inject real implementations into the 5 handler stubs
- `pending_approval` channel wired and ready for Plan 02 `/approve` flow
- `operator_pause` DB column exists; Plan 03 `/pause` and `/resume` handlers can UPDATE it immediately
- No blockers for Plans 02 or 03

---
*Phase: 07-telegram-bot*
*Completed: 2026-04-10*
