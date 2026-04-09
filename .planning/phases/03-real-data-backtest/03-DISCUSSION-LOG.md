# Phase 3: Real-Data Backtest - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-09
**Phase:** 03-real-data-backtest
**Areas discussed:** Tick replay granularity, GBM path, Fee calculation method, CLI redesign

---

## Tick Replay Granularity

| Option | Description | Selected |
|--------|-------------|----------|
| Every raw tick | Replay each pool_ticks row in time order — highest fidelity | ✓ |
| Hourly buckets | Aggregate rows by hour — 10-100x fewer rows, some intraday detail lost | |
| Daily buckets | One price per day, matches GBM output structure | |

**User's choice:** Every raw tick

---

| Option | Description | Selected |
|--------|-------------|----------|
| On every replayed tick | Call should_rebalance() on each tick row | ✓ |
| Once per day | Check at last tick of each calendar day | |

**User's choice:** On every replayed tick

---

| Option | Description | Selected |
|--------|-------------|----------|
| Keep DayResult — aggregate ticks to daily summary | Output schema unchanged, print_results() unchanged | ✓ |
| New TickResult — one entry per DB tick | Richer but potentially millions of rows | |
| Configurable --output-granularity flag | Most flexible, deferred to backlog | |

**User's choice:** Keep DayResult — aggregate ticks to daily summary

---

## GBM Path

| Option | Description | Selected |
|--------|-------------|----------|
| Keep as --synthetic flag (Recommended) | GBM retained, DB mode is default | ✓ |
| Remove GBM entirely | Cleaner codebase, loses synthetic sanity check | |

**User's choice:** Keep as --synthetic flag

---

| Option | Description | Selected |
|--------|-------------|----------|
| Same command, --synthetic flag | One command, two modes | ✓ |
| Separate subcommand | backtest db / backtest sim — cleaner separation | |

**User's choice:** Same command, --synthetic flag

---

## Fee Calculation Method

| Option | Description | Selected |
|--------|-------------|----------|
| fee_growth_global delta × simulated liquidity | Real protocol fees, most accurate | ✓ |
| Volume-estimate (same as GBM) | Simple, consistent with GBM, daily_volume user-supplied | |
| Claude's discretion | Let planner decide | |

**User's choice:** fee_growth_global delta × simulated liquidity

---

| Option | Description | Selected |
|--------|-------------|----------|
| Derive price from sqrt_price column | price = (sqrt_price / 2^64)^2 | ✓ |
| Derive price from tick_current column | price = 1.0001^tick_current | |

**User's choice:** Derive price from sqrt_price column

---

| Option | Description | Selected |
|--------|-------------|----------|
| Approximate: global delta × liquidity_share | (position_liq / pool_liq) × fee_growth_delta | ✓ |
| Exact: store fee_growth_inside separately | Requires additional schema columns — out of Phase 3 scope | |
| Claude's discretion | Let planner decide approximation strategy | |

**User's choice:** Approximate: global delta × liquidity_share

---

## CLI Redesign

| Option | Description | Selected |
|--------|-------------|----------|
| YYYY-MM-DD | Simple, matches ROADMAP.md example, UTC start-of-day | ✓ |
| RFC3339 full datetime | More precise, verbose for CLI | |

**User's choice:** YYYY-MM-DD

---

| Option | Description | Selected |
|--------|-------------|----------|
| Exit with error (clear message) | Non-zero exit, no silent fallback | |
| Fall back to GBM automatically | Could mask misconfiguration | |
| Exit with error + hint to --synthetic | Clear error + suggests alternative | ✓ |

**User's choice:** Exit with error + hint to --synthetic

---

| Option | Description | Selected |
|--------|-------------|----------|
| --liquidity <u128> CLI flag | User supplies raw liquidity value | |
| --capital only, derive share | Rough approximation from capital/TVL | |
| Claude's discretion | Let planner decide input mechanism | ✓ |

**User's choice:** Claude's discretion

---

## Claude's Discretion

- Position liquidity input mechanism for fee calculation
- Rust module structure for tick_reader
- fee_growth_global u128 wrapping handling

## Deferred Ideas

- Per-tick/hourly output granularity (--output-granularity flag)
- Exact fee_growth_inside tracking (v2 scope)
