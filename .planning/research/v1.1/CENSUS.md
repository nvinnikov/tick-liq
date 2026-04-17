# Pool Census: Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE

> Phase 6 deliverable — generated 2026-04-17 via Dune MCP

## Pool Identity

| Field | Value | Source |
|-------|-------|--------|
| Pool address | `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` | On-chain / Dune |
| DEX | Orca Whirlpool | `whirlpool_solana.whirlpool_call_initializepool` |
| Token A | wSOL (`So11111111111111111111111111111111111111112`) | Pool init tx |
| Token B | USDC (`EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`) | Pool init tx |
| Fee tier | **0.04%** (4 basis points) | tick spacing = 4 |
| Tick spacing | 4 | Pool init tx |
| Created | 2023-06-30 06:20:58 UTC | Pool init tx `36oqZY...` |
| Funder | `DjDsi34mSB66p2nhBL6YvhbcLtZbkGfNybFeLDjJqxJW` | Pool init tx |

This is the **SOL/USDC 0.04% pool** — the tightest fee tier on Orca Whirlpool, designed for professional market makers and high-frequency strategies.

> **TVL note**: TVL not available via Dune decoded tables. Cross-check via Orca UI or DexScreener for current TVL.

---

## Data Coverage Limitation

**Dune Solana indexing lag: 74 days as of 2026-04-17.**

The `whirlpool_solana` decoded tables have data through **2026-02-04** only. No data exists for the period 2026-02-05 through 2026-04-17.

| Table | Min date | Max date | Total rows |
|-------|----------|----------|-----------|
| `whirlpool_call_increaseliquidity` | 2022-03-10 | 2026-02-04 | 34,109,988 |
| `whirlpool_call_increaseliquidityv2` | (same range) | 2026-02-04 | — |
| `whirlpool_call_collectfees` | (same range) | 2026-02-04 | — |

**Implication**: Census is complete for the pool's full lifetime through 2026-02-04. The most recent ~74 days of activity are missing. For Phase 7 (Active Maker Filter) focusing on the last 90 days, adjust the window to `2025-11-06 to 2026-02-04` (90 days ending at max available date).

---

## Census Summary

| Metric | Value |
|--------|-------|
| Total unique LP addresses (all-time through 2026-02-04) | **57,057** |
| LP addresses active in last available 90 days (Nov 6, 2025 – Feb 4, 2026) | ~5,200 (v1) + ~222 (v2), some overlap |
| Total `increase_liquidity` events | 62,756 (last 90d) / 34M+ (all-time) |
| Total `collect_fees` events | 42,773 (last 90d) |

---

## Dune Queries

| Query | Purpose | URL |
|-------|---------|-----|
| Data coverage check | Verify pool has data, date ranges | https://dune.com/queries/7331925 |
| **Full LP census** | All LP addresses with lifetime stats | https://dune.com/queries/7331964 |
| Total unique LP count | UNION deduplication count | https://dune.com/queries/7331982 |
| Pool init info | Token pair, tick spacing, creation date | https://dune.com/queries/7331984 |

### Census SQL

```sql
-- Pool Census: Orca Whirlpool Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE
-- Data coverage: through 2026-02-04 (Dune Solana indexing lag)

WITH pool_addr AS (
    SELECT 'Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE' AS pool
),

inc_v1 AS (
    SELECT
        call_tx_signer                    AS lp_address,
        account_position                  AS position,
        call_block_time                   AS ts,
        CAST(liquidityAmount AS DOUBLE)   AS liquidity
    FROM whirlpool_solana.whirlpool_call_increaseliquidity, pool_addr
    WHERE account_whirlpool = pool
),

inc_v2 AS (
    SELECT
        call_tx_signer                    AS lp_address,
        account_position                  AS position,
        call_block_time                   AS ts,
        CAST(liquidityAmount AS DOUBLE)   AS liquidity
    FROM whirlpool_solana.whirlpool_call_increaseliquidityv2, pool_addr
    WHERE account_whirlpool = pool
),

all_inc AS (
    SELECT * FROM inc_v1
    UNION ALL
    SELECT * FROM inc_v2
),

lp_stats AS (
    SELECT
        lp_address,
        COUNT(DISTINCT position)    AS position_count,
        COUNT(*)                    AS increase_tx_count,
        SUM(liquidity)              AS total_liquidity_added,
        MIN(ts)                     AS first_seen,
        MAX(ts)                     AS last_seen
    FROM all_inc
    GROUP BY lp_address
),

fee_stats AS (
    SELECT
        account_positionAuthority   AS lp_address,
        COUNT(*)                    AS fee_collect_count,
        COUNT(DISTINCT account_position) AS positions_with_fees
    FROM whirlpool_solana.whirlpool_call_collectfees, pool_addr
    WHERE account_whirlpool = pool
    GROUP BY account_positionAuthority
)

SELECT
    l.lp_address,
    l.position_count,
    l.increase_tx_count,
    l.total_liquidity_added,
    COALESCE(f.fee_collect_count, 0)      AS fee_collect_count,
    COALESCE(f.positions_with_fees, 0)    AS positions_with_fees,
    l.first_seen,
    l.last_seen,
    DATE_DIFF('day', l.first_seen, l.last_seen) AS active_days
FROM lp_stats l
LEFT JOIN fee_stats f ON l.lp_address = f.lp_address
ORDER BY l.total_liquidity_added DESC
```

---

## Top LP Addresses (by total liquidity added, all-time)

| Rank | LP Address | Positions | Increase Txns | Total Liquidity | Fee Collects | Active Days | Notes |
|------|-----------|-----------|--------------|----------------|-------------|-------------|-------|
| 1 | `882DFRCi...` | 668 | 594,519 | 6.15e18 | 4,321 | 24 | Ultra-high-freq bot, 24 days |
| 2 | `Dh6mgdhy...` | 3 | 56,735 | 5.64e17 | 0 | 61 | High-freq, no fee collects |
| 3 | `FfarNfcL...` | 2 | 16,512 | 2.79e17 | 0 | 22 | High-freq, no fee collects |
| 4 | `6ZUeThQ9...` | 10,409 | 10,409 | 2.14e17 | 10,409 | 12 | Grid bot (1:1 pos:tx), 12 days |
| 5 | `5fDyjPr1...` | 182 | 3,792 | 1.86e17 | 1,976 | 20 | Active rebalancer |
| 6 | `C479QA85...` | 45 | 86 | 8.27e16 | 62 | 361 | Long-term passive LP |
| 7 | `GtomzVbj...` | 2,665 | 65,222 | 4.98e16 | 13,107 | 675 | Long-running active maker (**key candidate**) |
| 8 | `4Tfv29MR...` | 585 | 685 | 4.88e16 | 655 | 308 | Moderate activity |
| 9 | `DFRPGM4A...` | 361 | 1,502 | 3.89e16 | 5,639 | 264 | Active, many fee collects |
| 10 | `HAWK3BVn...` | 60,301 | 225,688 | 3.79e16 | 0 | 748 | Bot — 60k positions, no fee collects (unusual) |

**Notable patterns visible in top 10:**
- `882DFRCi`: 594,519 increase txns in 24 days = ~24,800 txns/day. Extreme bot.
- `HAWK3BVn`: 60,301 positions, 225,688 txns over 748 days but **0 fee collects** — likely auto-compounds or different collection mechanism.
- `GtomzVbj`: 675 days active, 65,222 txns, 13,107 fee collects — best "active MM" candidate for Phase 8 deep-dive.
- `6ZUeThQ9`: position_count == increase_tx_count (10,409) — one liquidity add per position, classic grid pattern.

---

## Dune Tables Reference

Key tables in `whirlpool_solana` schema:

| Table | Purpose | Key Columns |
|-------|---------|------------|
| `whirlpool_call_increaseliquidity` | LP adds liquidity (v1) | `account_whirlpool`, `call_tx_signer`, `account_position`, `liquidityAmount`, `call_block_date` |
| `whirlpool_call_increaseliquidityv2` | LP adds liquidity (v2) | Same as v1 |
| `whirlpool_call_decreaseliquidity` | LP removes liquidity (v1) | `account_whirlpool`, `call_tx_signer`, `account_position`, `liquidityAmount` |
| `whirlpool_call_decreaseliquidityv2` | LP removes liquidity (v2) | Same as v1 |
| `whirlpool_call_collectfees` | LP collects fees | `account_whirlpool`, `account_positionAuthority`, `account_position` |
| `whirlpool_call_collectfeesv2` | LP collects fees (v2) | Same as v1 |
| `whirlpool_call_openposition` | Open new position | `account_whirlpool`, `account_owner`, `tickLowerIndex`, `tickUpperIndex` |
| `whirlpool_call_closeposition` | Close position | `account_positionAuthority` (no `account_whirlpool`!) |
| `whirlpool_call_initializepool` | Pool creation | `account_whirlpool`, `account_tokenMintA`, `account_tokenMintB`, `tickSpacing` |
| `whirlpool_call_swap` / `swapv2` | Swaps (not LP activity) | — |

**Important**: `closeposition` does NOT have `account_whirlpool` — cannot filter closes by pool directly. Join via `account_position` from `openposition`.

---

## Data Limitations

1. **Dune lag**: 74-day gap (through 2026-02-04). Missing the most recent quarter of activity.
2. **No fee amounts**: `collectfees` table records the call but not the token amounts collected. Actual fee revenue requires cross-referencing token transfer logs.
3. **`closeposition` gap**: Close events can't be filtered by pool without a join to `openposition`.
4. **Liquidity units**: `liquidityAmount` is in raw Whirlpool liquidity units (uint256), not USD. Conversion requires current price.
5. **v1/v2 overlap**: Both `increaseliquidity` and `increaseliquidityv2` can be called by the same wallet; UNION (not UNION ALL) required for unique LP count.

---

## Alternative / Supplemental Sources

For data beyond 2026-02-04:
- **Helius RPC** (`getSignaturesForAddress` + `getTransaction`): Can reconstruct any address's transaction history in real-time. Required for Phase 8 deep-dive on a specific maker.
- **Orca API / UI**: Real-time pool stats and TVL at app.orca.so.
- **DexScreener / Birdeye**: Volume and TVL time series (useful for Phase 9 sizing).
- **Flipside Crypto**: Alternative Solana analytics platform, may have more recent data.

---

## Reproducibility

To reproduce the census:
1. Open https://dune.com/queries/7331964
2. Click "Run" — no parameters needed
3. Result set = all LP addresses with lifetime stats through 2026-02-04
4. Address set is deterministic (no randomness in SQL)

> The address set will be byte-identical on re-runs as long as Dune's data coverage doesn't extend further (which would add new rows).
