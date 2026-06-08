# Orca Whirlpool Pool Research Framework — Design Spec

**Date:** 2026-04-19
**Status:** Approved (v1)
**Default pool:** `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` (Orca Whirlpool, Solana)

## Goal

Build a reusable Dune-based research framework that profiles a single Orca Whirlpool pool along three axes: trade flow, liquidity-provider behavior, and competitive concentration. The output feeds into our LP-sizing strategy for the tick-liq automated LP manager — specifically, deciding (a) what trade sizes our deposited liquidity should be sized to absorb, and (b) what share of in-range liquidity we would represent against existing LPs.

The framework starts with one pool but every query is parameterized so the same artifacts can be re-run against any other Whirlpool address with no code changes.

## Non-goals (v1)

- No price-impact / slippage modeling. The end-state question — "for a given deposit size in a given range, what share of trade flow do we capture and what fees do we earn?" — requires reading `sqrtPriceX64` before/after each swap and reconstructing per-swap effective liquidity. Mentioned here as a planned follow-up; the user may execute it directly.
- No comparative analysis across multiple pools. Single-pool focus until the framework is proven.
- No on-chain execution. This is a Dune-only research workstream.

## Data sources (Dune)

Verified available on 2026-04-19:

- `dex_solana.trades` — curated swap spell. Columns relevant: `block_time`, `block_month` (partition), `project`, `amount_usd`, `fee_usd`, `fee_tier`, `trader_id`, `tx_id`, `outer_instruction_index`, `inner_instruction_index`, `token_bought_vault`, `token_sold_vault`. Filterable to Whirlpool via `project = 'whirlpool'`. **Does not carry the pool address directly** — pool identity is reached via vault addresses or by joining to decoded swap call tables.
- `whirlpool_solana.whirlpool_call_swap` and `whirlpool_call_swap_v2` — decoded swap calls. Carry the whirlpool pubkey as an explicit account argument. Used as the pool-identity bridge.
- `whirlpool_solana.whirlpool_evt_liquidityincreased` and `whirlpool_evt_liquiditydecreased` — emitted on every position-size change. Carry liquidity delta and parent pool.
- `whirlpool_solana.whirlpool_call_open_position`, `whirlpool_evt_positionopened`, `whirlpool_call_decrease_liquidity`, `whirlpool_evt_liquidityrepositioned` — position lifecycle events.

## Time windows

Tiered by query class:

- **7 days** for trade-size analysis. Trade-size distributions can shift with volatility regimes; a tight window is honest about the current regime and cheap to scan (~1 partition).
- **30 days** for LP behavior. Position open/close cycles run on day-to-week timescales; 30 days gives enough events to be statistically meaningful while staying within 1–2 partitions.

Both are query parameters (`{{days}}`), not hard-coded.

## Parameterization

Every query exposes:

- `{{pool_address}}` (text, default `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE`) — the whirlpool pubkey to filter on.
- `{{days}}` (number, default 7 or 30 depending on query class) — lookback window.

Every query MUST include `block_month >= date_trunc('month', now() - interval '{{days}}' day)` in addition to `block_time >= now() - interval '{{days}}' day` so partition pruning kicks in. Without the partition filter the trade table is prohibitively expensive.

## Filter strategy

For swap queries that need both pool identity AND USD enrichment:

1. Build a CTE from `whirlpool_call_swap` UNION ALL `whirlpool_call_swap_v2` filtered on the `whirlpool` account argument equal to `{{pool_address}}`. Project `(tx_id, outer_instruction_index, inner_instruction_index, block_time)`.
2. Join to `dex_solana.trades` on those instruction-index keys.
3. Apply partition filter on both sides.

For LP-event queries: filter the decoded event tables directly on the `whirlpool` account argument — no join needed.

## Queries

### Q1 — Volume + fee summary

**Window:** parameterized; will be run once at 7d and once at 30d.
**Granularity:** one row per UTC day.
**Columns:** `day`, `swap_count`, `volume_usd`, `fee_usd`, `distinct_traders`, `mean_trade_usd`, `median_trade_usd`, `fee_tier`.
**Purpose:** baseline pool-activity profile. Reveals whether the pool has consistent flow or is spiky.

### Q2 — Trade size distribution

**Window:** 7 days.
**Two output sets in the same query:**

1. Percentiles of `amount_usd`: p10, p25, p50, p75, p90, p95, p99 — single row.
2. Histogram with fixed buckets: `<$10`, `$10–100`, `$100–1k`, `$1k–10k`, `$10k–100k`, `>$100k`. Per bucket: swap count, volume share %, fee share %.

**Purpose:** directly answers "what trade sizes do we want to target." The histogram in particular tells us where fee revenue concentrates by trade size — a small number of large trades may dominate fees, or a long tail of small trades may.

### Q3 — LP event activity

**Window:** 30 days.
**Granularity:** one row per UTC day.
**Columns:** `day`, `positions_opened`, `positions_closed`, `increase_liquidity_count`, `decrease_liquidity_count`, `liquidity_repositioned_count`.
**Purpose:** measures LP churn and rebalancing intensity. High `liquidity_repositioned` activity means active LPs are competing on range tightness — that's directly relevant to our rebalance-engine design.

### Q4 — LP concentration

**Window:** 30 days.
**Granularity:** one row per LP owner address.
**Columns:** `owner`, `position_count`, `net_liquidity_delta`, `first_seen`, `last_seen`.
**Aggregation:** sum of liquidity-increased events minus liquidity-decreased events, grouped by owner (resolved from position pubkey via `whirlpool_call_open_position`).
**Output:** sorted descending by `net_liquidity_delta`, top 50.
**Purpose:** reveals whether the pool is dominated by a few whales (hard to compete with) or a long tail (easier to take share). Top-N table is a competitive map.

## Cost budget

- 7-day queries on `dex_solana.trades` with partition pruning: ~$0.20–0.50 in Dune credits each.
- 30-day queries: ~$0.50–1.50 each.
- Decoded `whirlpool_*` tables are smaller; cost is dominated by the trades table.
- Total v1 first run: ~$2–5 in credits. Will run `getUsage` before execution to confirm headroom.

## Execution order

1. Verify pool exists and resolve token mints — read 1 row from `whirlpool_call_swap` filtered on the pool address. Confirms the address is a valid Whirlpool and surfaces the token pair for sanity-checking later USD figures.
2. Q1 at 30 days (broad baseline first).
3. Q2 at 7 days.
4. Q3 at 30 days.
5. Q4 at 30 days.
6. Repeat Q1 at 7 days for direct comparability with Q2.

Each query is saved as a temp Dune query so it can be re-executed against other pools by changing the `{{pool_address}}` parameter.

## Deliverables

For the user, after execution:

- A short written summary of each query's findings.
- For each query, a link to the saved Dune query so the user can rerun or modify.
- An interpretation paragraph addressing the original strategy questions: minimum viable deposit size, expected fee yield range, and competitive density.

## Follow-up (v2, not in scope here)

Price-impact / market-share modeling. Requires:

- Reading `sqrtPriceX64` before/after each swap from the swap call accounts.
- Reconstructing per-swap effective in-range liquidity.
- Computing slippage and back-solving "if our $X liquidity were in range at this swap, what fee would we have captured?"

The user indicated they may build this out directly. Mentioned here so the v1 work doesn't preclude it (the v1 queries already establish the per-swap join keys v2 will need).
