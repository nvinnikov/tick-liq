---
phase: quick-260411-qjf
plan: "01"
subsystem: bot/queries
tags: [bugfix, sql, reporting, pnl]
dependency_graph:
  requires: []
  provides: [correct-24h-report]
  affects: [src/bot/queries.rs]
tech_stack:
  added: []
  patterns: [MAX-MIN delta aggregation for cumulative time-series fields]
key_files:
  created: []
  modified:
    - src/bot/queries.rs
    - src/math/fees.rs
decisions:
  - Use MAX()-MIN() delta pattern for all three cumulative pnl_history fields
metrics:
  duration: "~10 minutes"
  completed: "2026-04-11"
  tasks_completed: 1
  tasks_total: 1
  files_changed: 2
---

# Quick Task 260411-qjf: Fix Fees Double-Counting in 24H Report — Summary

**One-liner:** Replaced SUM() with MAX()-MIN() delta in query_24h_report so the 24H report shows actual fee/IL/PnL earned over the period instead of summing thousands of identical cumulative snapshots.

## What Was Done

### Task 1: Fix 24H report SQL — replace SUM with MAX-MIN delta

**File:** `src/bot/queries.rs` — `query_24h_report()`

The `pnl_history` table stores cumulative snapshots (not incremental rows). The prior query used `SUM()` across all rows in the 24h window, which multiplied the final cumulative value by the row count (e.g., $0.0082 actual fees appeared as ~$8.90 when 2705 rows were summed).

**Fix:** Changed all three cumulative aggregations to delta form:

```sql
-- Before (wrong: sums cumulative snapshots)
COALESCE(SUM(fees_earned), 0.0) AS total_fees,
COALESCE(SUM(il_usd), 0.0) AS total_il,
COALESCE(SUM(net_pnl), 0.0) AS total_net_pnl,

-- After (correct: delta over window)
COALESCE(MAX(fees_earned) - MIN(fees_earned), 0.0) AS total_fees,
COALESCE(MAX(il_usd) - MIN(il_usd), 0.0) AS total_il,
COALESCE(MAX(net_pnl) - MIN(net_pnl), 0.0) AS total_net_pnl,
```

Price fields (`MIN(price) FILTER`, `MAX(price) FILTER`) and `COUNT(*)` were already correct and remain unchanged. `ReportData` struct and `commands.rs` consumer required no changes.

**Commit:** `42f0467`

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed pre-existing clippy::precedence lint in fees.rs**
- **Found during:** Task 1 verification (clippy -D warnings)
- **Issue:** `a_lo * b_lo >> 64` has ambiguous operator precedence; clippy flagged it as an error under `-D warnings`, blocking the required clippy check
- **Fix:** Added explicit parentheses: `(a_lo * b_lo) >> 64` — no behavior change, purely cosmetic
- **Files modified:** `src/math/fees.rs`
- **Commit:** `42f0467` (same commit)

## Verification Results

- `cargo build` — passed
- `cargo clippy -- -D warnings` — passed
- Grep confirms: no `SUM(fees_earned)`, `SUM(il_usd)`, or `SUM(net_pnl)` in queries.rs

## Known Stubs

None.

## Threat Flags

None — no new trust boundaries introduced. The query already used bind parameters; this change only affects aggregation logic.

## Self-Check: PASSED

- `src/bot/queries.rs` exists and contains `MAX(fees_earned) - MIN(fees_earned)`
- `42f0467` commit exists in git log
