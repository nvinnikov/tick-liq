# Phase 3: Real-Data Backtest - Context

**Gathered:** 2026-04-09
**Status:** Ready for planning

<domain>
## Phase Boundary

`backtest` command reads actual `pool_ticks` rows from TimescaleDB for a given pool address and date range, replaying every tick to compute P&L. The GBM simulator is retained behind a `--synthetic` flag. Output schema (`BacktestResult` / `Vec<DayResult>`) is unchanged — internal per-tick replay, daily rollup for output. No schema changes to `pool_ticks`.

</domain>

<decisions>
## Implementation Decisions

### Tick Replay
- **D-01:** Replay every raw `pool_ticks` row in chronological order — no downsampling or aggregation before replay.
- **D-02:** Call `strategy::should_rebalance()` on every replayed tick (not end-of-day). Same trigger logic as shadow mode.
- **D-03:** Output is `Vec<DayResult>` (daily rollup) — ticks are processed individually internally, then aggregated to calendar-day summaries before populating `DayResult`. Existing `print_results()` and `BacktestResult` struct are unchanged.

### GBM Path Retention
- **D-04:** Keep the GBM synthetic backtest. The existing `backtest::run()` function stays. DB mode is the default path when `--pool` is provided. `--synthetic` flag activates GBM with existing params.
- **D-05:** Same `backtest` command, two modes:
  - DB mode: `cargo run -- backtest --pool <ADDR> --from <DATE> --to <DATE> [--capital X] [--fee-bps X] [--tick-spacing X] [--rebalance]`
  - GBM mode: `cargo run -- backtest --synthetic --entry-price X --price-lower X --price-upper X ...` (all existing flags unchanged)

### Fee Calculation (DB Mode)
- **D-06:** Compute real fee accrual from `fee_growth_global` delta between consecutive ticks.
  - Formula: `fees_tick = (fee_growth_global_a[t+1] - fee_growth_global_a[t]) × position_liquidity / pool_liquidity`
  - Use `pool_ticks.liquidity` as `pool_liquidity` for the share denominator.
  - Position liquidity input: **Claude's discretion** — planner decides the best mechanism (CLI flag, derivation from capital, or other).
- **D-07:** Price per tick is derived from `sqrt_price` column: `price = (sqrt_price_u128 as f64 / 2^64)^2`. Matches how the watch loop computes price.

### CLI Design
- **D-08:** `--from` and `--to` accept `YYYY-MM-DD` format, interpreted as UTC start-of-day.
- **D-09:** If no `pool_ticks` rows exist for the requested pool + date range: exit with a clear error and suggest `--synthetic` as an alternative. Example: `"No pool_ticks data for pool <ADDR> between <FROM> and <TO>. Run 'watch' to accumulate data, or use --synthetic for a GBM simulation."`
- **D-10:** DB-mode flags needed: `--pool <ADDR>`, `--from <DATE>`, `--to <DATE>`, `--capital` (retained), `--fee-bps` (retained), `--tick-spacing` (retained), `--rebalance` (retained). GBM-only flags (`--volatility`, `--seed`, `--daily-volume`, `--position-volume-share`, `--entry-price`, `--price-lower`, `--price-upper`, `--days`) are only used when `--synthetic` is present.

### Claude's Discretion
- Exact mechanism for supplying position liquidity in DB mode (CLI flag, derive from capital/pool TVL, or read from positions table).
- Rust module structure — whether `storage::tick_reader` is a new file or extends `storage/mod.rs`.
- How to handle `fee_growth_global` overflow (u128 wrapping) between ticks.

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Existing backtest engine (replace/extend this)
- `src/backtest/mod.rs` — `BacktestParams`, `BacktestResult`, `DayResult`, `run()`, `print_results()`. DB mode must produce the same `BacktestResult` struct.

### CLI entry point
- `src/main.rs` lines ~112-150 — `Commands::Backtest` enum variant (all current GBM flags). New `--pool`, `--from`, `--to`, `--synthetic` flags go here.
- `src/main.rs` lines ~1082-1111 — `Commands::Backtest` match arm (dispatch to `backtest::run()` or new DB path).

### Storage layer (read from this)
- `src/storage/schema.sql` — `pool_ticks` table definition: `time`, `pool_address`, `slot`, `tick_current`, `sqrt_price NUMERIC(80,0)`, `liquidity NUMERIC(80,0)`, `fee_growth_global_a NUMERIC(80,0)`, `fee_growth_global_b NUMERIC(80,0)`.
- `src/storage/mod.rs` — `run_migrations()`, `PgPool` setup.
- `src/storage/writer.rs` — `PoolTick` struct (matches DB columns, use as reference for reader struct).

### CLMM math (use for price derivation)
- `src/math/` — IL calculator (`compute_il`), tick↔price conversion. Phase 3 must use same math as watch loop.

### Strategy (rebalance signal)
- `src/strategy/mod.rs` — `should_rebalance(tick_current, tick_lower, tick_upper, net_pnl, &cfg)` — call on every replayed tick.

### Requirements
- `.planning/REQUIREMENTS.md` — BACKTEST-01 through BACKTEST-03.

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `backtest::DayResult` and `backtest::BacktestResult` — preserve exact field names and types; only the data source changes.
- `backtest::price_to_tick(price, tick_spacing)` — reuse in DB replay for rebalance signal.
- `strategy::should_rebalance()` — call per tick, same as GBM loop and shadow mode.
- `math::il::compute_il(entry_price, price, lower, upper)` — same IL formula, driven by derived price per tick.
- `storage::writer::PoolTick` — mirror this struct for a `storage::tick_reader` counterpart.

### Established Patterns
- `anyhow::Result` for all error paths — no `unwrap()`.
- `sqlx` non-macro query pattern (no compile-time `DATABASE_URL`) — see `storage/writer.rs` for the pattern to follow in the reader.
- `PgPool` passed as `&PgPool` parameter — same injection pattern for the backtest command.
- NUMERIC(80,0) → Rust: stored as decimal string in DB, parsed as `u128` in Rust.

### Integration Points
- `main.rs` `Commands::Backtest` match arm — add `pool`, `from`, `to`, `synthetic` branches here.
- `storage/mod.rs` — add `tick_reader` module export alongside `writer`.

</code_context>

<specifics>
## Specific Implementation Notes

- `--synthetic` retains all existing GBM flags unchanged. No breaking change to current backtest invocation.
- `fee_growth_global` is u128 X64 fixed-point; delta between rows must handle u128 wrapping (values can decrease if they wrapped).
- `price = (sqrt_price as f64 / 2^64)^2` — same derivation used in the watch loop. Keep consistent.
- The approximation `fee_growth_delta × (position_liquidity / pool_liquidity)` is intentionally simplified — exact fee_growth_inside requires tick array data not in schema. Note this as a known approximation in code comment.

</specifics>

<deferred>
## Deferred Ideas

- Per-tick or hourly output granularity (`--output-granularity` flag) — noted for backlog.
- Exact fee_growth_inside tracking (requires storing per-position tick array data during watch) — v2 / ANAL-02 scope.
- Historical tick import from Birdeye/Flipside — explicitly out of scope (PROJECT.md).

</deferred>

---

*Phase: 03-real-data-backtest*
*Context gathered: 2026-04-09 via /gsd-discuss-phase 3*
