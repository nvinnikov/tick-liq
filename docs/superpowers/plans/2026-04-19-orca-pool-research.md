# Orca Pool Research Framework — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Execute four parameterized Dune queries against Orca Whirlpool pool `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` to produce a reusable competitive-research framework: pool volume/fee profile (7d & 30d), trade-size distribution (7d), LP event activity (30d), and LP concentration (30d).

**Architecture:** Each query is built incrementally — schema verification, then a small probe query (LIMIT 5) to validate filters and join keys, then the full parameterized query saved to Dune as a temp query for re-use against other pools. SQL is also checked into the repo at `docs/superpowers/research/dune-queries/orca-pool/` so the framework survives Dune-side deletions. Findings are written to a single results doc.

**Tech Stack:** Dune MCP (`searchTables` with `includeSchema`, `createDuneQuery`, `executeQueryById`, `getExecutionResults`, `getUsage`), DuneSQL (Trino-based dialect).

**Spec:** `docs/superpowers/specs/2026-04-19-orca-pool-research-design.md`

---

## Conventions

- **Pool address parameter:** every saved Dune query uses parameter `pool_address` (text), default `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE`.
- **Days parameter:** every saved Dune query uses parameter `days` (number), default per query.
- **Partition pruning:** every query MUST include `block_month >= date_trunc('month', now() - interval '{{days}}' day)` alongside `block_time >= now() - interval '{{days}}' day`. Without this the trades table is ruinously expensive.
- **Pool-identity filter for swaps:** filter the decoded `whirlpool_solana.whirlpool_call_swap` (and `whirlpool_call_swap_v2`) table on the `account_whirlpool` column. Then join to `dex_solana.trades` on `(call_tx_id = tx_id, call_outer_instruction_index = outer_instruction_index, call_inner_instruction_index = inner_instruction_index)` to enrich with USD value.
- **Pool-identity filter for LP events:** filter the decoded `whirlpool_solana.whirlpool_evt_*` and `whirlpool_call_*` tables on `account_whirlpool` directly — no join needed.
- **Column-name caveat:** column names below are the standard Dune Solana convention (`account_<arg_name>`, `call_block_time`, `call_tx_id`, `call_outer_instruction_index`, `call_inner_instruction_index`, `evt_block_time`, `evt_block_month`). Task 1 verifies the actual names; if any differ, fix them inline before proceeding to subsequent tasks. Treat any divergence as a one-time correction across the plan, not per-task.
- **Saved Dune queries:** create as `is_temp: true` (they aren't dashboard-quality, just rerunnable). Capture the returned `query_id` in the results doc.
- **SQL files in repo:** store the canonical SQL at `docs/superpowers/research/dune-queries/orca-pool/<name>.sql`. Header comment in each file lists the parameters and a one-line description.
- **Results capture:** after each query executes, append findings to `docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md`. Include execution_id, query_id, key numbers, and a one-paragraph interpretation.
- **Commits:** one commit per task (after the query SQL is saved + results captured). Co-author per repo convention.

---

### Task 0: Pre-flight — credit check + result file scaffolding

**Files:**
- Create: `docs/superpowers/research/dune-queries/orca-pool/.gitkeep`
- Create: `docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md`

- [ ] **Step 1: Check Dune credit headroom**

Call `mcp__dune__getUsage`. Confirm at least ~10 credits worth of headroom remain (full plan budget is ~$2–5; 2× safety margin). If insufficient, stop and report.

- [ ] **Step 2: Create directory structure**

Run:
```bash
mkdir -p docs/superpowers/research/dune-queries/orca-pool
touch docs/superpowers/research/dune-queries/orca-pool/.gitkeep
```

- [ ] **Step 3: Create results doc with header**

Write `docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md`:

```markdown
# Orca Pool Research Results — Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE

**Date:** 2026-04-19
**Plan:** docs/superpowers/plans/2026-04-19-orca-pool-research.md
**Spec:** docs/superpowers/specs/2026-04-19-orca-pool-research-design.md

## Pool identity

(Filled in Task 1.)

## Q1 — Volume + fee summary

(Filled in Task 2.)

## Q2 — Trade size distribution (7d)

(Filled in Task 3.)

## Q3 — LP event activity (30d)

(Filled in Task 4.)

## Q4 — LP concentration (30d)

(Filled in Task 5.)

## Synthesis

(Filled in Task 6.)
```

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/plans/2026-04-19-orca-pool-research.md \
        docs/superpowers/specs/2026-04-19-orca-pool-research-design.md \
        docs/superpowers/research/dune-queries/orca-pool/.gitkeep \
        docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md
git commit -m "$(cat <<'EOF'
docs(research): scaffold Orca pool research framework

Add spec, plan, results scaffold for pool Czfq3x...
LP-sizing analysis: trade-size + LP behavior over 7d/30d windows.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 1: Schema verification + pool sanity check

**Files:**
- Create: `docs/superpowers/research/dune-queries/orca-pool/probe.sql`
- Modify: `docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md` (Pool identity section)

- [ ] **Step 1: Fetch schemas for the four key tables**

Call `mcp__dune__searchTables` four times (or in one batch where possible) with `includeSchema: true`:

1. `query: "whirlpool call swap"`, `blockchains: ["solana"]`, schemas filtered to `whirlpool_solana` — capture column list for `whirlpool_call_swap` and `whirlpool_call_swap_v2`.
2. `query: "whirlpool liquidity increased"`, `blockchains: ["solana"]` — capture `whirlpool_evt_liquidityincreased`.
3. `query: "whirlpool liquidity decreased"`, `blockchains: ["solana"]` — capture `whirlpool_evt_liquiditydecreased`.
4. `query: "whirlpool open position"`, `blockchains: ["solana"]` — capture `whirlpool_call_open_position`.

For each table, write down: the account-argument column for the pool address (expected `account_whirlpool`), the time column (`call_block_time` or `evt_block_time`), the partition column (`block_month`), and the join keys (`call_tx_id` / `evt_tx_id`, instruction indices).

If any column name diverges from the conventions in the "Conventions" section above, update the SQL in subsequent tasks before running it.

- [ ] **Step 2: Write probe query SQL**

Create `docs/superpowers/research/dune-queries/orca-pool/probe.sql`:

```sql
-- Probe: confirm pool exists, surface token mints, validate filter columns.
-- Parameters: pool_address (text)
SELECT
  account_whirlpool AS pool_address,
  account_token_mint_a AS token_mint_a,
  account_token_mint_b AS token_mint_b,
  call_block_time     AS first_swap_seen
FROM whirlpool_solana.whirlpool_call_swap
WHERE account_whirlpool = '{{pool_address}}'
  AND block_month >= date_trunc('month', now() - interval '7' day)
  AND call_block_time >= now() - interval '7' day
ORDER BY call_block_time ASC
LIMIT 1
```

NOTE: if Step 1 revealed that the swap call doesn't carry `account_token_mint_a/b` (some Anchor IDLs only expose vault accounts), fall back to:

```sql
SELECT
  account_whirlpool       AS pool_address,
  account_token_vault_a   AS token_vault_a,
  account_token_vault_b   AS token_vault_b,
  call_block_time         AS first_swap_seen
FROM whirlpool_solana.whirlpool_call_swap
WHERE account_whirlpool = '{{pool_address}}'
  AND block_month >= date_trunc('month', now() - interval '7' day)
  AND call_block_time >= now() - interval '7' day
ORDER BY call_block_time ASC
LIMIT 1
```

If `whirlpool_call_swap` returns zero rows in 7 days, also UNION ALL the same query against `whirlpool_call_swap_v2`.

- [ ] **Step 3: Create + execute probe as a Dune query**

Call `mcp__dune__createDuneQuery` with `name: "tick-liq probe — Orca pool sanity check"`, `is_temp: true`, the SQL from Step 2, and `parameters: [{ "key": "pool_address", "type": "text", "value": "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE" }]`.

Then call `mcp__dune__executeQueryById` with the returned `query_id` and `performance: "medium"`.

Then call `mcp__dune__getExecutionResults` with the returned `execution_id`.

Expected: 1 row containing `pool_address` matching the input and the token mints (or vaults).

If 0 rows: pool may be inactive in last 7d — extend window to 30d in the probe SQL and rerun. If still 0 rows, the pool may be a different program (Raydium, etc.) — STOP and report.

- [ ] **Step 4: Resolve token mints if probe returned vaults**

If the probe returned vault addresses instead of mints, run a quick lookup query against `tokens_solana.fungible` joined via the vault → mint relationship, OR look up the mint from the token vault account directly. Capture both mint addresses and their symbols.

(Skip this step if Step 3 already returned `token_mint_a/b`.)

- [ ] **Step 5: Record findings in results doc**

Update the `## Pool identity` section of the results doc with:

```markdown
- Program: Orca Whirlpool
- Pool address: Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE
- Token A: <mint> (<symbol>)
- Token B: <mint> (<symbol>)
- First swap seen in last 7d (UTC): <timestamp>
- Probe Dune query ID: <query_id>
- Probe execution ID: <execution_id>
```

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/research/dune-queries/orca-pool/probe.sql \
        docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md
git commit -m "$(cat <<'EOF'
docs(research): probe Orca pool, capture identity

Sanity-check pool address, surface token pair.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Q1 — Volume + fee summary (7d & 30d)

**Files:**
- Create: `docs/superpowers/research/dune-queries/orca-pool/q1-volume-fees.sql`
- Modify: `docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md` (Q1 section)

- [ ] **Step 1: Write Q1 SQL**

Create `docs/superpowers/research/dune-queries/orca-pool/q1-volume-fees.sql`:

```sql
-- Q1: Daily volume + fee summary for a single Whirlpool pool.
-- Parameters: pool_address (text), days (number)
WITH pool_swaps AS (
  SELECT
    call_tx_id                    AS tx_id,
    call_outer_instruction_index  AS outer_instruction_index,
    call_inner_instruction_index  AS inner_instruction_index,
    call_block_time               AS block_time
  FROM whirlpool_solana.whirlpool_call_swap
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  SELECT
    call_tx_id, call_outer_instruction_index,
    call_inner_instruction_index, call_block_time
  FROM whirlpool_solana.whirlpool_call_swap_v2
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND call_block_time >= now() - interval '{{days}}' day
),
enriched AS (
  SELECT
    date_trunc('day', t.block_time) AS day,
    t.amount_usd,
    t.fee_usd,
    t.fee_tier,
    t.trader_id
  FROM dex_solana.trades t
  INNER JOIN pool_swaps p
    ON  t.tx_id = p.tx_id
    AND t.outer_instruction_index = p.outer_instruction_index
    AND t.inner_instruction_index = p.inner_instruction_index
  WHERE t.project = 'whirlpool'
    AND t.block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND t.block_time >= now() - interval '{{days}}' day
)
SELECT
  day,
  COUNT(*)                                          AS swap_count,
  SUM(amount_usd)                                   AS volume_usd,
  SUM(fee_usd)                                      AS fee_usd,
  COUNT(DISTINCT trader_id)                         AS distinct_traders,
  AVG(amount_usd)                                   AS mean_trade_usd,
  approx_percentile(amount_usd, 0.5)                AS median_trade_usd,
  ARBITRARY(fee_tier)                               AS fee_tier
FROM enriched
GROUP BY 1
ORDER BY 1 ASC
```

- [ ] **Step 2: Probe with LIMIT 5 and days=1**

Before saving as a parameterized query, validate the SQL by running it inline (or as a throwaway query) with `days=1` and a `LIMIT 5` appended. Confirm: the result shape has all expected columns, day values look right, swap_count > 0.

If the join returns zero rows: the instruction-index keys may need adjustment. Most likely cause is that `dex_solana.trades` uses different inner-instruction indexing for nested swaps — check by joining on `(tx_id, outer_instruction_index)` only and inspecting how many rows match per swap call. Adjust accordingly before proceeding.

- [ ] **Step 3: Save as parameterized Dune query**

Call `mcp__dune__createDuneQuery`:
- `name`: `"tick-liq Q1 — Whirlpool daily volume + fees"`
- `is_temp`: `true`
- `query`: SQL from Step 1
- `parameters`:
  ```json
  [
    {"key": "pool_address", "type": "text", "value": "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"},
    {"key": "days", "type": "number", "value": "30"}
  ]
  ```

Capture `query_id`.

- [ ] **Step 4: Execute at 30 days**

Call `mcp__dune__executeQueryById` with the `query_id`, `performance: "medium"`, and `query_parameters` overriding `days` to `"30"`. Capture `execution_id`.

Call `mcp__dune__getExecutionResults` with the `execution_id`. Capture all 30 rows.

- [ ] **Step 5: Execute at 7 days**

Same call as Step 4 but with `days` parameter `"7"`. Capture both execution_id and 7 result rows.

- [ ] **Step 6: Record findings**

Update the `## Q1 — Volume + fee summary` section of the results doc with:

```markdown
**Dune query ID:** <query_id>
**Executions:** 30d=<execution_id_30>, 7d=<execution_id_7>

### 30-day window
- Total volume: $<X>
- Total fees: $<Y> (effective fee rate: Y/X = ...%)
- Daily swap count: mean <N>, range [<min>, <max>]
- Distinct traders (30d): <Z>
- Fee tier: <bps>

### 7-day window
- Total volume: $<X>
- Total fees: $<Y>
- Daily swap count: mean <N>, range [<min>, <max>]
- Distinct traders (7d): <Z>

### Observation
<one-paragraph interpretation: is the pool active and consistent, or spiky? Does 7d look like a representative sample of 30d?>
```

- [ ] **Step 7: Commit**

```bash
git add docs/superpowers/research/dune-queries/orca-pool/q1-volume-fees.sql \
        docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md
git commit -m "$(cat <<'EOF'
docs(research): Q1 — Whirlpool daily volume + fees

7d and 30d snapshots for pool Czfq3x...

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Q2 — Trade size distribution (7d)

**Files:**
- Create: `docs/superpowers/research/dune-queries/orca-pool/q2-trade-size.sql`
- Modify: `docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md` (Q2 section)

- [ ] **Step 1: Write Q2 SQL**

Create `docs/superpowers/research/dune-queries/orca-pool/q2-trade-size.sql`:

```sql
-- Q2: Trade-size distribution for a single Whirlpool pool.
-- Returns two result sets via UNION ALL with a discriminator column.
-- Parameters: pool_address (text), days (number, default 7)
WITH pool_swaps AS (
  SELECT call_tx_id AS tx_id,
         call_outer_instruction_index AS outer_instruction_index,
         call_inner_instruction_index AS inner_instruction_index,
         call_block_time AS block_time
  FROM whirlpool_solana.whirlpool_call_swap
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  SELECT call_tx_id, call_outer_instruction_index,
         call_inner_instruction_index, call_block_time
  FROM whirlpool_solana.whirlpool_call_swap_v2
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND call_block_time >= now() - interval '{{days}}' day
),
enriched AS (
  SELECT t.amount_usd, t.fee_usd
  FROM dex_solana.trades t
  INNER JOIN pool_swaps p
    ON  t.tx_id = p.tx_id
    AND t.outer_instruction_index = p.outer_instruction_index
    AND t.inner_instruction_index = p.inner_instruction_index
  WHERE t.project = 'whirlpool'
    AND t.block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND t.block_time >= now() - interval '{{days}}' day
    AND t.amount_usd IS NOT NULL
),
percentiles AS (
  SELECT
    'percentiles'                          AS row_kind,
    'all'                                  AS bucket,
    COUNT(*)                               AS swap_count,
    SUM(amount_usd)                        AS volume_usd,
    SUM(fee_usd)                           AS fee_usd,
    approx_percentile(amount_usd, 0.10)    AS p10,
    approx_percentile(amount_usd, 0.25)    AS p25,
    approx_percentile(amount_usd, 0.50)    AS p50,
    approx_percentile(amount_usd, 0.75)    AS p75,
    approx_percentile(amount_usd, 0.90)    AS p90,
    approx_percentile(amount_usd, 0.95)    AS p95,
    approx_percentile(amount_usd, 0.99)    AS p99
  FROM enriched
),
bucketed AS (
  SELECT
    CASE
      WHEN amount_usd < 10        THEN '1: <$10'
      WHEN amount_usd < 100       THEN '2: $10-100'
      WHEN amount_usd < 1000      THEN '3: $100-1k'
      WHEN amount_usd < 10000     THEN '4: $1k-10k'
      WHEN amount_usd < 100000    THEN '5: $10k-100k'
      ELSE                             '6: >$100k'
    END AS bucket,
    amount_usd, fee_usd
  FROM enriched
),
histogram AS (
  SELECT
    'histogram'              AS row_kind,
    bucket,
    COUNT(*)                 AS swap_count,
    SUM(amount_usd)          AS volume_usd,
    SUM(fee_usd)             AS fee_usd,
    CAST(NULL AS DOUBLE)     AS p10,
    CAST(NULL AS DOUBLE)     AS p25,
    CAST(NULL AS DOUBLE)     AS p50,
    CAST(NULL AS DOUBLE)     AS p75,
    CAST(NULL AS DOUBLE)     AS p90,
    CAST(NULL AS DOUBLE)     AS p95,
    CAST(NULL AS DOUBLE)     AS p99
  FROM bucketed
  GROUP BY bucket
)
SELECT * FROM percentiles
UNION ALL
SELECT * FROM histogram
ORDER BY row_kind, bucket
```

- [ ] **Step 2: Save as parameterized Dune query**

Call `mcp__dune__createDuneQuery`:
- `name`: `"tick-liq Q2 — Whirlpool trade-size distribution"`
- `is_temp`: `true`
- `query`: SQL from Step 1
- `parameters`:
  ```json
  [
    {"key": "pool_address", "type": "text", "value": "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"},
    {"key": "days", "type": "number", "value": "7"}
  ]
  ```

Capture `query_id`.

- [ ] **Step 3: Execute at 7 days**

Call `mcp__dune__executeQueryById` with `query_id`, `performance: "medium"` (default `days=7`). Capture `execution_id`.

Call `mcp__dune__getExecutionResults`. Expect 7 rows total: 1 percentiles row + 6 histogram bucket rows (assuming all buckets hit; some may be empty).

- [ ] **Step 4: Record findings**

Update the `## Q2 — Trade size distribution (7d)` section of the results doc with:

```markdown
**Dune query ID:** <query_id>
**Execution ID:** <execution_id>

### Percentiles (USD)
| p10 | p25 | p50 | p75 | p90 | p95 | p99 |
|---|---|---|---|---|---|---|
| ... | ... | ... | ... | ... | ... | ... |

### Histogram
| Bucket | Swap count | Volume USD | Volume share | Fee USD | Fee share |
|---|---|---|---|---|---|
| <$10 | ... | ... | ...% | ... | ...% |
| $10-100 | ... | ... | ...% | ... | ...% |
| $100-1k | ... | ... | ...% | ... | ...% |
| $1k-10k | ... | ... | ...% | ... | ...% |
| $10k-100k | ... | ... | ...% | ... | ...% |
| >$100k | ... | ... | ...% | ... | ...% |

### Observation
<one paragraph: where does fee revenue concentrate? Long tail of small trades, or a few whales?
What trade-size band should our liquidity be sized to absorb?>
```

Volume share and fee share are computed locally from the bucket totals (`bucket_volume_usd / sum_of_all_bucket_volumes`).

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/research/dune-queries/orca-pool/q2-trade-size.sql \
        docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md
git commit -m "$(cat <<'EOF'
docs(research): Q2 — Whirlpool trade-size distribution

7d percentiles + USD-band histogram for pool Czfq3x...

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Q3 — LP event activity (30d)

**Files:**
- Create: `docs/superpowers/research/dune-queries/orca-pool/q3-lp-events.sql`
- Modify: `docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md` (Q3 section)

- [ ] **Step 1: Write Q3 SQL**

Create `docs/superpowers/research/dune-queries/orca-pool/q3-lp-events.sql`:

```sql
-- Q3: Daily LP event activity for a single Whirlpool pool.
-- Parameters: pool_address (text), days (number, default 30)
WITH all_events AS (
  SELECT date_trunc('day', call_block_time) AS day, 'open' AS kind
  FROM whirlpool_solana.whirlpool_call_open_position
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  SELECT date_trunc('day', call_block_time) AS day, 'close' AS kind
  FROM whirlpool_solana.whirlpool_call_close_position
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  SELECT date_trunc('day', call_block_time) AS day, 'increase' AS kind
  FROM whirlpool_solana.whirlpool_call_increase_liquidity
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  SELECT date_trunc('day', call_block_time), 'increase'
  FROM whirlpool_solana.whirlpool_call_increase_liquidity_v2
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  SELECT date_trunc('day', call_block_time), 'decrease'
  FROM whirlpool_solana.whirlpool_call_decrease_liquidity
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  SELECT date_trunc('day', call_block_time), 'decrease'
  FROM whirlpool_solana.whirlpool_call_decrease_liquidity_v2
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  SELECT date_trunc('day', evt_block_time), 'reposition'
  FROM whirlpool_solana.whirlpool_evt_liquidityrepositioned
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND evt_block_time >= now() - interval '{{days}}' day
)
SELECT
  day,
  SUM(CASE WHEN kind = 'open'       THEN 1 ELSE 0 END) AS positions_opened,
  SUM(CASE WHEN kind = 'close'      THEN 1 ELSE 0 END) AS positions_closed,
  SUM(CASE WHEN kind = 'increase'   THEN 1 ELSE 0 END) AS increase_liquidity,
  SUM(CASE WHEN kind = 'decrease'   THEN 1 ELSE 0 END) AS decrease_liquidity,
  SUM(CASE WHEN kind = 'reposition' THEN 1 ELSE 0 END) AS liquidity_repositioned
FROM all_events
GROUP BY day
ORDER BY day ASC
```

NOTE: if Task 1 schema verification revealed that `whirlpool_call_close_position` doesn't exist (some Anchor versions only emit close via a separate `close_position_with_token_extensions`), do another `searchTables` call for `query: "whirlpool close position"` and adjust the union list. Same caveat for `whirlpool_evt_liquidityrepositioned` — it's the newest event in the IDL and may not exist if the pool hasn't been touched by the latest program version; if a search returns no such table, drop that branch from the UNION.

- [ ] **Step 2: Save as parameterized Dune query**

Call `mcp__dune__createDuneQuery`:
- `name`: `"tick-liq Q3 — Whirlpool LP event activity"`
- `is_temp`: `true`
- `query`: SQL from Step 1
- `parameters`:
  ```json
  [
    {"key": "pool_address", "type": "text", "value": "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"},
    {"key": "days", "type": "number", "value": "30"}
  ]
  ```

Capture `query_id`.

- [ ] **Step 3: Execute at 30 days**

Call `mcp__dune__executeQueryById` with `query_id`, `performance: "medium"`. Capture `execution_id`.

Call `mcp__dune__getExecutionResults`. Expect up to 30 rows (one per day; days with zero events will be missing).

- [ ] **Step 4: Record findings**

Update the `## Q3 — LP event activity (30d)` section of the results doc with:

```markdown
**Dune query ID:** <query_id>
**Execution ID:** <execution_id>

### 30-day totals
| Event | Count |
|---|---|
| Positions opened | ... |
| Positions closed | ... |
| Increase liquidity | ... |
| Decrease liquidity | ... |
| Liquidity repositioned | ... |

### Daily averages and peaks
- Mean opens/day: <N>
- Mean repositions/day: <N>
- Peak day for repositions: <date> (<count> events)

### Observation
<one paragraph: are LPs passive or active? High reposition counts signal active competitors that respond
to price moves — informs whether our rebalance engine needs to be aggressive to compete.>
```

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/research/dune-queries/orca-pool/q3-lp-events.sql \
        docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md
git commit -m "$(cat <<'EOF'
docs(research): Q3 — Whirlpool LP event activity

30d daily counts of open/close/increase/decrease/reposition
for pool Czfq3x...

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Q4 — LP concentration (30d)

**Files:**
- Create: `docs/superpowers/research/dune-queries/orca-pool/q4-lp-concentration.sql`
- Modify: `docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md` (Q4 section)

- [ ] **Step 1: Write Q4 SQL**

Create `docs/superpowers/research/dune-queries/orca-pool/q4-lp-concentration.sql`:

```sql
-- Q4: LP concentration for a single Whirlpool pool.
-- Owner is the signer (funder) of the open_position call;
-- liquidity delta is sum of liquidityIncreased - liquidityDecreased per position.
-- Parameters: pool_address (text), days (number, default 30)
WITH positions AS (
  SELECT
    account_position             AS position_pubkey,
    account_funder               AS owner,
    call_block_time              AS opened_at
  FROM whirlpool_solana.whirlpool_call_open_position
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND call_block_time >= now() - interval '{{days}}' day
),
increases AS (
  SELECT
    account_position AS position_pubkey,
    SUM(CAST(liquidity_amount AS DOUBLE)) AS total_increase
  FROM whirlpool_solana.whirlpool_evt_liquidityincreased
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND evt_block_time >= now() - interval '{{days}}' day
  GROUP BY account_position
),
decreases AS (
  SELECT
    account_position AS position_pubkey,
    SUM(CAST(liquidity_amount AS DOUBLE)) AS total_decrease
  FROM whirlpool_solana.whirlpool_evt_liquiditydecreased
  WHERE account_whirlpool = '{{pool_address}}'
    AND block_month >= date_trunc('month', now() - interval '{{days}}' day)
    AND evt_block_time >= now() - interval '{{days}}' day
  GROUP BY account_position
),
position_net AS (
  SELECT
    p.owner,
    p.position_pubkey,
    p.opened_at,
    COALESCE(i.total_increase, 0) - COALESCE(d.total_decrease, 0) AS net_liquidity_delta
  FROM positions p
  LEFT JOIN increases i USING (position_pubkey)
  LEFT JOIN decreases d USING (position_pubkey)
)
SELECT
  owner,
  COUNT(*)                  AS position_count,
  SUM(net_liquidity_delta)  AS net_liquidity_delta_total,
  MIN(opened_at)            AS first_seen,
  MAX(opened_at)            AS last_seen
FROM position_net
GROUP BY owner
ORDER BY net_liquidity_delta_total DESC
LIMIT 50
```

NOTE on event column names: the `liquidity` field in `whirlpool_evt_liquidityincreased` may be named `liquidity`, `liquidity_amount`, or `delta_liquidity` depending on Dune's IDL parsing. Task 1 schema verification step should have surfaced the actual name; substitute it for `liquidity_amount` in the SQL above.

NOTE on `account_position` vs `account_position_authority`: the position pubkey is the NFT mint that represents the position. The Anchor account name might be `account_position` or `account_position_mint`. Verify in Task 1.

NOTE on owner: `account_funder` is the signer of `open_position`. This is the "creator" of the position but may not be the current holder if the position NFT was transferred. For v1 we accept this as a proxy for the owner; flag it in the observation paragraph.

- [ ] **Step 2: Save as parameterized Dune query**

Call `mcp__dune__createDuneQuery`:
- `name`: `"tick-liq Q4 — Whirlpool LP concentration"`
- `is_temp`: `true`
- `query`: SQL from Step 1
- `parameters`:
  ```json
  [
    {"key": "pool_address", "type": "text", "value": "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"},
    {"key": "days", "type": "number", "value": "30"}
  ]
  ```

Capture `query_id`.

- [ ] **Step 3: Execute at 30 days**

Call `mcp__dune__executeQueryById` with `query_id`, `performance: "medium"`. Capture `execution_id`. Then `getExecutionResults`. Expect ≤50 rows.

- [ ] **Step 4: Record findings**

Update the `## Q4 — LP concentration (30d)` section of the results doc with:

```markdown
**Dune query ID:** <query_id>
**Execution ID:** <execution_id>

### Top 10 LPs by net liquidity contributed (30d)
| Rank | Owner (truncated) | Position count | Net liquidity delta | First seen | Last seen |
|---|---|---|---|---|---|
| 1 | abc...xyz | ... | ... | ... | ... |
| ... | | | | | |

### Concentration metrics
- Top 1 LP share of total net liquidity added: ...%
- Top 5 LP share: ...%
- Top 10 LP share: ...%
- Total distinct LPs (in top-50): ...

### Caveats
- "Owner" here is the position-NFT funder (creator), not the current holder. Position NFTs can be
  transferred. For v1 this is treated as a proxy.
- "Net liquidity delta" is in raw `liquidity` units (Whirlpool's L), not USD. Cross-LP comparison is
  valid; absolute interpretation requires conversion via current price + tick range.

### Observation
<one paragraph: is this pool whale-dominated or long-tail? If top-1 holds >50%, we're competing with
a single big LP. If long-tail, our deposit can take a meaningful share more easily.>
```

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/research/dune-queries/orca-pool/q4-lp-concentration.sql \
        docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md
git commit -m "$(cat <<'EOF'
docs(research): Q4 — Whirlpool LP concentration

30d top-50 LPs ranked by net liquidity added
for pool Czfq3x...

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Synthesis report

**Files:**
- Modify: `docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md` (Synthesis section)

- [ ] **Step 1: Write the synthesis**

Update the `## Synthesis` section with three subsections:

```markdown
### Pool profile
<2-3 sentences cross-referencing Q1 and Q2: how active is this pool, what's the typical trade size,
what's the daily fee revenue we'd be competing for a share of.>

### Competitive landscape
<2-3 sentences cross-referencing Q3 and Q4: how concentrated is the LP base, how active are LPs at
rebalancing, what does that imply about competition intensity.>

### Recommended next step for the LP-sizing model
<2-3 sentences: what's the minimum viable deposit given the trade-size distribution and LP concentration?
What's the realistic fee-yield range we should target? Should we proceed with v2 (price-impact / share-capture
modeling) for this pool, or is the pool too small/concentrated to be worth deeper analysis?>

### Reusing this framework for other pools
1. Replace `pool_address` parameter when re-executing the saved Dune queries (IDs in this doc).
2. Or copy the SQL from `docs/superpowers/research/dune-queries/orca-pool/` into a new Dune query.
3. Q1 supports `days` 1–60. Q2/Q3/Q4 cost scales linearly with `days`.

### v2 follow-up (out of scope here)
Price-impact / market-share modeling: read `sqrt_price_x64` before/after each swap from
`whirlpool_call_swap.account_*` accounts, reconstruct effective in-range liquidity, back-solve
"if our $X were in-range, what fee would we have captured." User indicated they may build this directly.
The v1 join-key extraction in Q1/Q2 (tx_id + instruction indices) is the same shape v2 will need.
```

- [ ] **Step 2: Final commit**

```bash
git add docs/superpowers/research/2026-04-19-orca-pool-Czfq3x-results.md
git commit -m "$(cat <<'EOF'
docs(research): Orca pool research synthesis

Cross-query synthesis + reuse instructions for other pools.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 3: Report to user**

Surface to the user:
- Path to results doc
- Five Dune query IDs (probe + Q1–Q4) so they can rerun in browser
- The synthesis recommendation in 2–3 sentences
