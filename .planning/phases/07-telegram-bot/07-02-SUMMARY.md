---
phase: 07-telegram-bot
plan: "02"
subsystem: bot
tags: [teloxide, telegram, proposal, approval, oneshot, tokio, rust]

# Dependency graph
requires:
  - phase: 07-01
    provides: BotState, spawn_bot, pending_approval channel, --telegram CLI flag
provides:
  - src/bot/proposal.rs with send_proposal() and await_approval()
  - /approve handler completing oneshot channel with chat_id auth gate
  - write_approval_skip() for DB logging of timeout/rejected skips
  - Telegram proposal gate wired into watch loop rebalance path
  - load_chat_id() reading TELEGRAM_CHAT_ID env var
affects: [07-03]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Proposal flow: send_proposal installs oneshot::Sender, await_approval parks on tokio::time::timeout"
    - "block_in_place pattern for async calls from sync Fn watch callback"
    - "Auth gate: msg.chat.id.0 != state.chat_id on all 5 command handlers (T-07-02)"
    - "One proposal at a time: send_proposal drops existing sender before installing new one (D-02)"

key-files:
  created:
    - src/bot/proposal.rs
  modified:
    - src/bot/mod.rs
    - src/bot/commands.rs
    - src/storage/writer.rs
    - src/main.rs

key-decisions:
  - "block_in_place used for send_proposal/await_approval in sync Fn callback (plan assumed async callback, actual type is sync Fn)"
  - "telegram_bot Option<Bot> created before closure as separate handle for proposal sends, distinct from dispatcher Bot in spawn_bot"
  - "load_chat_id() called twice (BotState construction + telegram_chat_id capture) — acceptable; only called at startup"

# Metrics
duration: 40min
completed: 2026-04-10T17:36:29Z
---

# Phase 07 Plan 02: Rebalance Proposal Flow Summary

**Telegram proposal-approval-timeout cycle: send_proposal() sends formatted message, await_approval() parks on oneshot with configurable timeout, /approve completes it, timeout/reject logs skip to DB and skips execution**

## Performance

- **Duration:** ~40 min
- **Started:** 2026-04-10T17:00:00Z
- **Completed:** 2026-04-10T17:36:29Z
- **Tasks:** 2
- **Files modified:** 5 (1 created, 4 modified)

## Accomplishments

- Created `src/bot/proposal.rs` with `send_proposal()` (installs oneshot::Sender, sends formatted Telegram message), `await_approval()` (parks on tokio::time::timeout), and `format_proposal()` (structured P&L proposal message)
- Added `chat_id: i64` field to `BotState` and `load_chat_id()` helper reading `TELEGRAM_CHAT_ID` env var
- Implemented `/approve` handler completing the oneshot channel; all 5 command handlers now have `msg.chat.id.0 != state.chat_id` auth gate (T-07-02 mitigation)
- Added `write_approval_skip()` to `storage/writer.rs` for parameterized DB logging of timeout/reject events
- Wired full proposal flow into watch loop: `telegram_bot`, `telegram_chat_id`, `approve_timeout_secs_val` captured before closure; approval gate inserted before `guard.submit()` using `block_in_place` pattern

## Task Commits

Each task was committed atomically:

1. **Task 1: Create proposal module, chat_id auth, write_approval_skip** — `1ec2742` (feat)
2. **Task 2: Wire proposal flow into watch loop rebalance path** — `bb82327` (feat)

## Files Created/Modified

- `src/bot/proposal.rs` — `send_proposal()`, `await_approval()`, `format_proposal()`, `ProposalData` struct
- `src/bot/mod.rs` — Added `pub mod proposal`, `chat_id: i64` field to `BotState`, `load_chat_id()` function
- `src/bot/commands.rs` — Full `/approve` handler with oneshot completion; chat_id auth gate on all 5 handlers
- `src/storage/writer.rs` — Added `write_approval_skip()` with parameterized SQL
- `src/main.rs` — `telegram_bot`/`telegram_chat_id` captures, `approve_timeout_secs_val`, full proposal gate before `guard.submit()`

## Decisions Made

- `block_in_place` used for `send_proposal`/`await_approval` calls: the watch callback is `Box<dyn Fn(serde_json::Value) + Send + 'static>` (sync), not async. The plan's description of it as "async context" was incorrect. The existing `write_pool_tick` call in the same callback already uses `block_in_place` for the same reason.
- `telegram_bot` is a separate `teloxide::Bot` handle created before the closure, distinct from the dispatcher's Bot. The dispatcher owns its own Bot handle; the watch loop needs a separate one for sending proposal messages directly.
- `load_chat_id()` called twice at startup (BotState + telegram_chat_id): acceptable because it is called only once at watch startup, not per-tick.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Used block_in_place for proposal/approval calls in sync callback**
- **Found during:** Task 2
- **Issue:** Plan stated "The watch loop rebalance path is already async — send_proposal and await_approval must be called with .await directly." However, `NotifyFn = Box<dyn Fn(serde_json::Value) + Send + 'static>` is a **sync** `Fn` closure. Calling `.await` directly is not possible.
- **Fix:** Used `tokio::task::block_in_place(|| Handle::current().block_on(...))` — the same pattern already used for `write_pool_tick` and `fetch_drift_margin_ratio` in the same callback.
- **Files modified:** src/main.rs
- **Commit:** `bb82327`

**2. [Rule 2 - Missing Critical] Added temporary #![allow(dead_code)] to proposal.rs for Task 1 commit**
- **Found during:** Task 1 verification
- **Issue:** `send_proposal` and `await_approval` are not called until Task 2 wires them; `cargo clippy -D warnings` rejects unused functions.
- **Fix:** Added `#![allow(dead_code)]` for the Task 1 commit, removed it in Task 2 after wiring.
- **Files modified:** src/bot/proposal.rs
- **Committed in:** `1ec2742` (added), `bb82327` (removed)

## Threat Surface Scan

No new network endpoints beyond what the plan's threat model covers.

- T-07-02 mitigated: `msg.chat.id.0 != state.chat_id` check on all 5 command handlers; unauthorized chat IDs are silently dropped with a warning log
- T-07-04 mitigated: `send_proposal` drops existing oneshot sender before installing a new one — only one proposal pending at a time; old sender is consumed on drop
- T-07-06 mitigated: `write_approval_skip` uses parameterized query — no user-supplied Telegram message content reaches SQL

## Self-Check

Files exist:
- src/bot/proposal.rs — created this session
- src/bot/mod.rs — modified this session
- src/bot/commands.rs — modified this session
- src/storage/writer.rs — modified this session
- src/main.rs — modified this session

Commits:
- 1ec2742 — feat(07-02): add proposal module, chat_id auth gate, write_approval_skip
- bb82327 — feat(07-02): wire proposal approval flow into watch loop rebalance path

## Self-Check: PASSED

Both commits exist, all files present, `cargo clippy -- -D warnings` exits 0.

---
*Phase: 07-telegram-bot*
*Completed: 2026-04-10*
