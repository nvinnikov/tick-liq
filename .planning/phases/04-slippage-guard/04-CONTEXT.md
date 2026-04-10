# Phase 4: Slippage Guard - Context

**Gathered:** 2026-04-10
**Status:** Ready for planning

<domain>
## Phase Boundary

A `strategy::slippage` module computes simulated price impact (in bps) for the trade that a rebalance would execute. A slippage gate sits between `should_rebalance()` returning `Rebalance` and `build_rebalance_plan()` being called. If impact exceeds `--max-slippage-bps` (default 50), the rebalance is aborted and the event is persisted to DB and logged. The gate is bypassed in shadow mode (no real capital at risk). Phase 4 does not execute transactions — it only gates the path to `build_rebalance_plan()`.

</domain>

<decisions>
## Implementation Decisions

### Trade size proxy
- Use **position value in USD** as the trade size passed to impact computation
- Compute: `position_value_usd = token_a_amount * price + token_b_amount` (from position state available at the call site)
- Pass `position_value_usd` to `strategy::slippage::check_slippage()` as the proxy for how much we'd move through the pool

### Abort logging destination
- Persist to **DB**: add a row to `shadow_rebalances` with `trigger_reason = 'slippage_abort'`
- Also emit `tracing::warn!` with structured fields (`impact_bps`, `threshold_bps`, `position_value_usd`)
- The DB write mirrors the Phase 2 shadow logging pattern — allows post-hoc analysis of how often slippage protection fires
- Fields on the abort row: `impact_bps` (f64), `threshold_bps` (u32), `position_value_usd` (f64); simulated_* fields are NULL

### Integration point
- **Separate gate** between `should_rebalance()` and `build_rebalance_plan()` — does NOT fold into `should_rebalance()`
- The watch loop call chain becomes:
  1. `strategy::should_rebalance()` → if `Rebalance`
  2. `strategy::slippage::check_slippage()` → if `SlippageAbort`, log + persist + skip `build_rebalance_plan()`
  3. `execution::build_rebalance_plan()` → if ok
- Mirrors the `ShadowGuard` separation-of-concerns pattern from Phase 2
- `should_rebalance()` signature is unchanged — no new args

### Config struct design
- Own **`SlippageConfig`** struct in `strategy::slippage` module:
  ```rust
  pub struct SlippageConfig {
      pub max_bps: u32, // default: 50
  }
  ```
- Parsed from `--max-slippage-bps` CLI flag in main.rs, validated at startup (reject 0 or > 10_000)
- Passed to the slippage gate separately from `RebalanceConfig`
- `RebalanceConfig` in `signal.rs` is unchanged

### Claude's Discretion
- Internal formula for converting `estimate_impact()` output to bps (algebraic inversion vs. direct computation)
- Exact field names on `shadow_rebalances` abort rows beyond the ones listed above
- Whether `check_slippage()` returns a `Result` or a dedicated enum (`SlippageOk | SlippageAbort { impact_bps }`)

</decisions>

<specifics>
## Specific Implementation Notes

- The project operates at ~$20-30k capital — 50bps default is calibrated for this size
- `estimate_impact()` in `math/impact.rs` is the existing math primitive; the slippage module wraps it
- Abort rows in `shadow_rebalances` use `trigger_reason = 'slippage_abort'` (new value alongside existing 'out_of_range' | 'il_threshold' | 'manual')
- Gate is per-rebalance-attempt, not per-tick — only runs when `should_rebalance()` returns `Rebalance`

</specifics>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Existing slippage math
- `src/math/impact.rs` — `estimate_impact(price, liquidity, target_pct, is_buy)` and `build_distribution()` — the math primitive to wrap

### Rebalance decision path (integration target)
- `src/strategy/signal.rs` — `should_rebalance()` and `RebalanceConfig` — upstream of the slippage gate
- `src/execution/rebalance.rs` — `build_rebalance_plan()` — downstream of the slippage gate
- `src/execution/shadow_guard.rs` — `ShadowGuard` — established pattern for intercepting at execution boundary

### Storage layer (for abort logging)
- `src/storage/writer.rs` — `shadow_rebalances` write path; abort rows extend this
- `src/storage/schema.sql` — `shadow_rebalances` table schema; `trigger_reason` column must accept 'slippage_abort'

### CLI wiring
- `src/main.rs` — `--max-slippage-bps` flag must be added here; watch loop is the call site for the new gate

### Requirements
- `.planning/REQUIREMENTS.md` — SLIPPAGE-01, SLIPPAGE-02, SLIPPAGE-03 definitions

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `math::impact::estimate_impact()` — already computes USD-to-move-price; slippage module inverts this to get bps for a given trade size
- `ShadowGuard` enum pattern — clean model for a gate that intercepts between two layers
- `shadow_rebalances` write path in `storage::writer` — can be extended for abort rows

### Established Patterns
- `anyhow::Error` for all error propagation (CLAUDE.md requirement — no `unwrap()` in production paths)
- `tracing::warn!` with structured fields for operational events
- Config structs passed as references to pure decision functions

### Integration Points
- `src/main.rs` watch loop: between `should_rebalance()` call and `build_rebalance_plan()` call — new slippage gate inserts here
- `src/main.rs` CLI arg parsing: `--max-slippage-bps` flag + startup validation

</code_context>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope.

</deferred>

---

*Phase: 04-slippage-guard*
*Context gathered: 2026-04-10 via /gsd-discuss-phase 4*
