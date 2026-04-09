# Phase 2: Shadow Mode - Context

**Gathered:** 2026-04-09
**Status:** Ready for planning
**Source:** /gsd-discuss-phase 2

<domain>
## Phase Boundary

Full rebalance decision logic (should_rebalance → build_rebalance_plan) runs on every tick without signing or submitting transactions. Every decision is logged to a `shadow_rebalances` DB table. The `--live` flag is hard-blocked until 14 days of shadow data with zero error-flagged rows. Phase 2 also wires real P&L computation (fees, IL, net) into pnl_history rows, replacing the 0.0 stubs from Phase 1.

</domain>

<decisions>
## Implementation Decisions

### --shadow / --live flag design
- Two mutually exclusive flags: `--shadow` (default for watch) and `--live`
- Shadow is the DEFAULT — running `watch` without any flag behaves as shadow mode
- `--live` requires explicit opt-in AND passing the hard DB gate check

### Shadow gate enforcement (LOCKED)
- Hard DB check, not a warning or config file
- Gate criteria: `SELECT MIN(created_at) FROM shadow_rebalances WHERE pool_address = $1` must be 14+ days before NOW()
- AND: `SELECT COUNT(*) FROM shadow_rebalances WHERE pool_address = $1 AND error_flag = true` must be 0
- If either criterion fails: process exits with a descriptive error message and non-zero exit code
- No override flag — the gate is unconditional

### Error definition for shadow gate (LOCKED)
- Any `anyhow::Error` propagated through the rebalance decision path sets `error_flag = true` on the current shadow row
- This includes strategy errors, data errors, and calculation failures — not just panics
- Errors are captured by wrapping the rebalance decision call in a match/map_err

### Shadow log schema: shadow_rebalances table (LOCKED)
Full simulation fields per row:
- `id` BIGSERIAL PRIMARY KEY
- `created_at` TIMESTAMPTZ NOT NULL DEFAULT NOW()
- `pool_address` TEXT NOT NULL
- `trigger_reason` TEXT NOT NULL — 'out_of_range' | 'il_threshold' | 'manual'
- `price` DOUBLE PRECISION NOT NULL — current price at decision time
- `simulated_range_width` DOUBLE PRECISION — new range width if rebalanced
- `simulated_fees_earned` DOUBLE PRECISION
- `simulated_il_usd` DOUBLE PRECISION
- `simulated_net_pnl` DOUBLE PRECISION
- `error_flag` BOOLEAN NOT NULL DEFAULT FALSE
- `error_message` TEXT — populated when error_flag = true

### Real P&L computation in Phase 2 (LOCKED)
- pnl_history rows must have real computed values from Phase 2 onward
- Use existing `strategy::` IL calculator and fee tracker
- Wire into `spawn_pnl_write` call site in main.rs watch loop
- Phase 1 stubs (0.0) are replaced — no backwards compat needed

### Rebalance wiring
- shadow mode calls full path: `strategy::should_rebalance()` → `execution::build_rebalance_plan()`
- build_rebalance_plan result is used to populate simulated_* fields in shadow_rebalances
- No transaction construction, no signing, no RPC submission in shadow mode
- `ShadowGuard` struct wraps the execution layer and intercepts at the point where a transaction would be submitted

### Claude's Discretion
- Exact Rust type for ShadowGuard (newtype wrapper, trait, or plain bool guard)
- Migration strategy for shadow_rebalances table (add to schema.sql + run_migrations)
- Tracing span structure for shadow rebalance decisions

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Existing rebalance logic (entry points)
- `src/main.rs` lines ~818-920 — `strategy::should_rebalance()` call site and `execution::build_rebalance_plan()` call site in watch loop
- `src/strategy/mod.rs` — `should_rebalance()` function
- `src/execution/mod.rs` — `build_rebalance_plan()` function

### Phase 1 storage layer (extend this)
- `src/storage/writer.rs` — `PoolTick`, `PnlSnapshot`, `write_pool_tick`, `spawn_pnl_write`
- `src/storage/schema.sql` — existing pool_ticks + pnl_history schema (add shadow_rebalances here)
- `src/storage/mod.rs` — `run_migrations()` entry point

### Requirements
- `.planning/REQUIREMENTS.md` — SHADOW-01 through SHADOW-04 definitions

</canonical_refs>

<specifics>
## Specific Implementation Notes

- Shadow is DEFAULT: `cargo run -- watch` == shadow mode. Requires `--live` to go live.
- Gate is per pool_address — different pools can have different shadow windows
- The shadow log fields include full simulated outcome to make the 2-week window analytically useful
- Real P&L computation is Phase 2 scope — don't defer to Phase 5

</specifics>

<deferred>
## Deferred Ideas

- Shadow mode web dashboard / report view — Phase 7 (Telegram /report covers this)
- Shadow mode comparison vs live mode (A/B) — out of scope for v1

</deferred>

---

*Phase: 02-shadow-mode*
*Context gathered: 2026-04-09 via /gsd-discuss-phase 2*
