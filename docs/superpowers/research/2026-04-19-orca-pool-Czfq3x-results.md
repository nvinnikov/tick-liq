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

(Filled in Task 2.)

## Q2 — Trade size distribution (7d)

(Filled in Task 3.)

## Q3 — LP event activity (30d)

(Filled in Task 4.)

## Q4 — LP concentration (30d)

(Filled in Task 5.)

## Synthesis

(Filled in Task 6.)
