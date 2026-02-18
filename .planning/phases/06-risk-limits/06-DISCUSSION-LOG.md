# Phase 6: Risk Limits - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-10
**Phase:** 06-risk-limits
**Areas discussed:** Drift margin ratio source, Risk check timing, IL pause / auto-resume logic, Drawdown close-all scope

---

## Drift Margin Ratio Source

| Option | Description | Selected |
|--------|-------------|----------|
| Stub it | Always report "margin ratio OK", log Drift CPI deferred. CLI flag parsed but check is no-op. | |
| Read-only account fetch | Add RPC fetch of Drift User account to get real margin ratio. No CPI — just deserialization. | ✓ |

**User's choice:** Read-only account fetch — Phase 6 adds real Drift margin ratio monitoring (no CPI).

**Follow-up: Drift account deserialization approach**

| Option | Description | Selected |
|--------|-------------|----------|
| Add drift-sdk / drift-cpi crate | Official typed struct + margin ratio calculation. | |
| Manual borsh layout | Hand-write field offsets. Brittle but no dependency. | |
| You decide | Claude picks approach that minimizes risk. | ✓ |

**User's choice:** Claude's discretion.

---

## Risk Check Timing

**Q: When should the risk monitor evaluate thresholds?**

| Option | Description | Selected |
|--------|-------------|----------|
| Every incoming tick | Fires on each WebSocket event, same as slippage check. Fast protection. | ✓ |
| Only at rebalance evaluation | Simpler, slower to detect breach between rebalances. | |
| Configurable interval | --risk-check-interval-secs flag. More flexible, more complex. | |

**User's choice:** Every incoming tick.

**Follow-up: Where in the tick loop?**

| Option | Description | Selected |
|--------|-------------|----------|
| Before rebalance signal check | Order: risk check → should_rebalance() → slippage → execute | |
| After P&L write, before rebalance | Order: pnl_history write → risk check → should_rebalance() | ✓ |

**User's choice:** After pnl_history write, before should_rebalance(). Ensures latest P&L is in DB before risk reads it.

---

## IL Pause / Auto-Resume Logic

**Q: How does rebalancing resume after IL drops below threshold?**

| Option | Description | Selected |
|--------|-------------|----------|
| Fully automatic | pause_flag cleared automatically when IL < --max-il on any tick. | ✓ |
| Manual operator action | Stays paused until /resume or restart. | |

**User's choice:** Fully automatic.

**Follow-up: Hysteresis?**

| Option | Description | Selected |
|--------|-------------|----------|
| No hysteresis — exact threshold | Resume as soon as IL < --max-il. Predictable. | ✓ |
| Add hysteresis buffer | Resume only at (--max-il * 0.9). Prevents flapping. | |

**User's choice:** No hysteresis. Exact threshold = pause threshold = resume threshold.

---

## Drawdown Close-All Scope

**Q: What does the halt sequence close?**

| Option | Description | Selected |
|--------|-------------|----------|
| LP position only — log hedge intent | OrcaExecutor close_position + collect_fees. Log Drift hedge skip. | ✓ |
| LP + attempt Drift hedge close stub | Close LP + hedge_close_stub() logs deferred. | |

**User's choice:** LP only. Drift hedge close logged as CRITICAL-level deferred note.

**Follow-up: What happens to the process?**

| Option | Description | Selected |
|--------|-------------|----------|
| Process exits (non-zero code) | std::process::exit(3) after LP close. Operator must restart. | |
| Process halts rebalancing but stays alive | Sets halt_flag in DB, continues logging ticks. | ✓ |

**User's choice:** Process stays alive. halt_flag = true in DB, rebalancing permanently suppressed.

**Follow-up: How to clear halt flag?**

| Option | Description | Selected |
|--------|-------------|----------|
| Manual DB clear or process restart | halt_flag in DB, survives restarts. Manual SQL UPDATE to clear. | ✓ |
| Process restart auto-clears | halt_flag in-memory only. Restart resumes trading. | |

**User's choice:** Manual DB clear required. halt_flag is persistent — restart does NOT auto-clear.

---

## Claude's Discretion

- Drift account deserialization crate vs. manual borsh
- Exact DB schema columns for risk_state table
- Rust struct layout for RiskState
- Tracing span structure for risk breaches

## Deferred Ideas

- Drift hedge close execution (needs LIVE-02)
- Telegram /resume command to clear halt_flag (Phase 7)
- CLI watch --clear-halt flag (potential Phase 7 addition)
