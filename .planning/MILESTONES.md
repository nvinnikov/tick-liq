# Milestones

## v1.0 MVP (Shipped: 2026-04-10)

**Phases completed:** 5 phases, 6 plans, 10 tasks

**Key accomplishments:**

- pnl_history writer with fire-and-forget spawn_pnl_write enabling non-blocking P&L recording per tick event (PERSIST-02, PERSIST-03)
- Wiring points in `src/main.rs`:
- Real P&L computation (replaces Phase 1 stubs):
- 1. [Rule 2 - Missing critical functionality] Added no-DB + --live exit gate
- One-liner:
- One-liner:
- One-liner:
- teloxide 0.13 bot module with 5 command stubs wired into watch as tokio task, operator_pause DB column, and pending_approval channel for rebalance approval flow
- Telegram proposal-approval-timeout cycle: send_proposal() sends formatted message, await_approval() parks on oneshot with configurable timeout, /approve completes it, timeout/reject logs skip to DB and skips execution
- One-liner:

---
