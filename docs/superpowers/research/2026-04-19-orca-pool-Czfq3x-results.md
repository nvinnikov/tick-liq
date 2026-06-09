# Orca Pool Research Results — Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE

**Date:** 2026-04-19
**Plan:** docs/superpowers/plans/2026-04-19-orca-pool-research.md
**Spec:** docs/superpowers/specs/2026-04-19-orca-pool-research-design.md

## Pool identity

- **Program:** Orca Whirlpool
- **Pool address:** `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE`
- **Pair:** WSOL / USDC
  - WSOL: `So11111111111111111111111111111111111111112`
  - USDC: `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`
- **Vaults:** `EUuUbDcafPrmVTD5M6qoJAoyyNbihBhugADAxRMn5he9` (vault_a), `2WLWEuKDgkDUccTpbwYp1GToYktiSB1cXvreHUwiSUVP` (vault_b)
- **Fee tier:** 0.0004 (= 4 bps)
- **First swap observed in 7d window:** 2026-04-12 10:26:56 UTC
- **Probe Dune query ID:** 7339553
- **Probe execution ID:** `01KPJMDKXW44G1ATVPT9140CN8`
- **Probe credit cost:** 4.33

### Schema corrections discovered during probe
- `dex_solana.trades` mint columns are `token_bought_mint_address` / `token_sold_mint_address` (NOT `token_bought_address` as the curated description suggested).
- Decoded `whirlpool_*` tables do NOT have `block_month`; only `call_block_time` and `call_block_date`. The `block_month` partition filter applies only to the `dex_solana.trades` spell.
- `whirlpool_call_swap` is a unified table with both camelCase AND snake_case columns aliased (no need for v2 union for swaps in v1).
- Liquidity-event tables (open/close/increase/decrease) come in 4 variants each: snake_case base, camelCase base, snake_case _v2, camelCase _v2. Q3/Q4 must UNION the snake_case base + snake_case _v2 (camelCase variants are duplicates of the same on-chain instruction; using only one naming family avoids double-counting).

## Q1 — Volume + fee summary

**Dune query ID:** 7339629
**Executions:** 30d=`01KPJMHZ5W46YZME3ZDYG7T43F`, 7d=`01KPJMMD1QNB48PYMHBTYAQ2HR`
**Credits:** 30d=7.81, 7d=2.106

### 30-day window (2026-03-20 → 2026-04-18)
- Total volume: **~$746.7M** ($510.9M excluding the 2026-04-01 outlier)
- Total fees: **~$298.7k** (matches 4 bps × volume)
- Total swaps: **456,805**
- Daily mean (excl. 4/1): swap_count ~14,026, volume ~$17.6M, fees ~$7.0k
- Daily range (excl. 4/1): swaps 5,065–25,301; volume $5.2M–$38.3M
- **Outlier:** 2026-04-01 had 48,168 swaps, $235.8M volume, $94.3k fees, 2,553 distinct traders, mean trade size $4,896 — clearly a major price-move day. Kept in dataset; flagged so it doesn't dominate averages.
- Distinct traders/day (typical): 400–900
- Daily median trade size: $495–$820
- Daily mean trade size: $983–$1,539 (excl. 4/1)
- Fee tier: **4 bps** consistent across all days

### 7-day window (2026-04-12 → 2026-04-18)
- Total volume: **~$119.4M**
- Total fees: **~$47.8k**
- Total swaps: **92,526**
- Daily mean: swap_count ~13,218, volume ~$17.1M, fees ~$6.8k
- Distinct traders/day: 322–728
- Daily median trade size: $552–$824
- Daily mean trade size: $1,191–$1,409

### Observation
Pool is extremely active — ~456k swaps and ~$747M volume in 30 days makes this one of the larger Orca Whirlpools. Activity is consistent (CV of daily volume ~30% excl. outlier), which means a 7-day window is a representative sample of "normal" days. The 4/1 spike (5× normal volume, 8× fees) is the type of event our fee-yield model should treat as upside, not baseline. Daily fee revenue at the pool level is in the **$5k–$10k range**, so an LP with ~1% in-range share is theoretically capturing $50–$100/day before IL.

## Q2 — Trade size distribution (7d)

(Filled in Task 3.)

## Q3 — LP event activity (30d)

(Filled in Task 4.)

## Q4 — LP concentration (30d)

(Filled in Task 5.)

## Synthesis

(Filled in Task 6.)
