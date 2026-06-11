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

**Dune query ID:** 7339637
**Execution ID:** `01KPJMRQ61W87DEP5T53WTCPPB`
**Credits:** 2.657
**Sample:** 92,525 swaps over 7d (matches Q1 to within 1 swap — null amount_usd filter)

### Percentiles (USD)
| p10 | p25 | p50 (median) | p75 | p90 | p95 | p99 |
|---|---|---|---|---|---|---|
| $159 | $440 | **$734** | $1,090 | $2,851 | $4,586 | $10,257 |

### Histogram
| Bucket | Swaps | % of swaps | Volume USD | % of volume | Fee USD | % of fees |
|---|---|---|---|---|---|---|
| <$10 | 2,145 | 2.32% | $4,531 | 0.004% | $1.81 | 0.004% |
| $10–100 | 5,094 | 5.51% | $388,312 | 0.33% | $155 | 0.33% |
| $100–1k | 56,246 | **60.79%** | $32.79M | 27.46% | $13,117 | 27.46% |
| **$1k–10k** | **28,063** | **30.33%** | **$68.57M** | **57.41%** | **$27,427** | **57.41%** |
| $10k–100k | 972 | 1.05% | $16.61M | 13.91% | $6,642 | 13.91% |
| >$100k | 5 | 0.005% | $1.07M | 0.90% | $428 | 0.90% |
| **Total** | **92,525** | 100% | **$119.43M** | 100% | **$47,771** | 100% |

### Observation
**The $1k–$10k bucket is where the fee revenue lives**: 30% of swaps generate 57% of fees. The $100–$1k bucket is the high-frequency long tail (61% of count, 27% of fees) — these are retail/aggregator-routed trades. Sub-$100 trades (~8% of count) contribute <0.4% of fees and look like noise (arb dust, fee spam, or rounding remainders from multi-hop routes).

For LP sizing the takeaway is clear: **size the position to comfortably absorb p95 trades (~$4.6k) without significant price-out-of-range risk**. Going for p99 ($10k+) is diminishing returns — the top decile of trade size only contributes ~15% of remaining fees, and the few >$100k whales are 5 trades over a week (likely AMM-aware market-making flow that doesn't ride a single LP anyway).

## Q3 — LP event activity (30d)

**Dune query ID:** 7339657
**Execution ID:** `01KPJN0P13TEEZ9MB2BMRZ0R14`
**Credits:** 0.143

### 30-day totals (2026-03-20 → 2026-04-18, excluding 4/19 partial day)
| Event | Count | Per-day mean |
|---|---|---|
| Positions opened | 3,872 | ~129 |
| Increase liquidity | 50,343 | ~1,678 |
| Decrease liquidity | 27,717 | ~924 |

### Daily ranges
- Positions opened: 41 (3/21) – 242 (4/18)
- Increase liquidity: 875 (3/20) – 2,554 (4/2)
- Decrease liquidity: 407 (3/20) – 1,613 (4/16)

### Activity ratios
- **Increase / Open ≈ 13** — each new position averages 13 increase events (active scaling/rebalancing)
- **Decrease / Open ≈ 7.2** — frequent partial withdrawals or rebalance-driven decreases
- Combined: ~80 LP-state-changing events/hour, every hour, for 30 days straight

### Caveats
- `whirlpool_call_close_position` lacks `account_whirlpool` (close instruction doesn't reference the pool); using `decrease_liquidity` count as close-activity proxy.
- `whirlpool_evt_liquidityrepositioned` (a newer event for atomic range reset) deferred — not all program versions emit it; would need a separate join.
- Snake_case + camelCase variants of the decoded tables are UNION'd and de-duplicated by `(tx_id, outer_idx, inner_idx, kind)` to avoid double-counting across IDL eras.

### Observation
This pool is **not run by passive retail LPs** — the increase-to-open ratio of 13× and the per-day cadence of ~80 events/hour are unmistakable bot signatures. Likely contributors: managed LP protocols (Kamino, Marginfi, Krystal-style auto-managers) and proprietary market-making bots that rebalance ranges on every meaningful price move. Our rebalance engine has to be in the same league or our positions will be out-of-range while competitors capture the in-range fees. **The cost of being slow on this pool is high.** Conversely, this also means the pool has been validated as profitable enough that sophisticated capital is willing to play — a positive signal for entering.

## Q4 — LP concentration (30d)

**Dune query ID:** 7339673
**Execution ID:** `01KPJN8W5K93P5BAZ1D7Z8047J`
**Credits:** 0.162
**Output:** top 50 LPs (by `position_authority` = signer of inc/dec calls) ranked by net liquidity delta.

### Top 10 LPs (net liquidity delta in raw Whirlpool L units)
| Rank | Owner | Inc count | Dec count | Net Δ (10^12 L) | Pattern |
|---|---|---|---|---|---|
| 1 | `2KovWEtY69R2LsmuvEGsj9UQNFJy5paPS1hbbQZzfq9U` | 127 | 56 | **534.2** | Active whale — 2× more inflows than outflows |
| 2 | `Dyn3TVbdtHrAjWXwQjGkdahSyC87gTdVYGAu3yAWduP`  | 2 | 0 | 151.4 | Passive whale entry (no withdraws) |
| 3 | `KVVrxqeYE6dbX4EotCgb6eU2wYSSQxB7SJgiyyy5c3R`  | 128 | 98 | 44.1 | Pure rebalance bot (gross flow 17,300 T) |
| 4 | `ByiAbN9MJhfQKGK5WJrfgko6XS88qqERQVRLWZTsvyTf` | 1 | 0 | 28.2 | Single-shot whale deposit |
| 5 | `86jxR1EavkbNdRnCW6Ar7fbNA18avxs4a5dhk3ghde4h` | 106 | 41 | 26.7 | Active manager |
| 6 | `74CkTXVFiqa12sGWFUsZUtXtWKA8AydLeYAx3PRFT5sa` | 2 | 1 | 26.2 | Mostly passive |
| 7 | `HdHZe1MvhGSDQ32BBc6AimGpCtGjhb7295akaPVckQ3s` | **3,642** | 536 | 20.2 | High-frequency bot (~121 inc/day) |
| 8 | `F3QqXMK8AiN8kP85fEZC9GbawLa7AvXQyZe7oTHwNgyZ` | 42 | 11 | 18.0 | |
| 9 | `BraiVsYAraGCuGp9HBkVMngTzdHYg5Ci65WxyB72iGbn` | 69 | 54 | 15.9 | |
| 10 | `EvjAwbehCugmFmzGHpezRg2xgExPNNVBP2UJYDWhPQTw` | 22 | 9 | 11.1 | |

### Concentration metrics (rough; sums based on top-50 returned)
- Approximate sum of net Δ across top-50: ~1,000 × 10^12 L
- **Top 1 share: ~53%** of top-50 net liquidity contributed
- Top 5 share: ~78%
- Top 10 share: ~88%
- Total distinct LPs in top-50: 50 (no truncation needed; tail is shallow)

### Two LP archetypes visible in the data
1. **Passive whales** — 1–3 increase events, 0 decreases. Likely large LPs depositing and holding through a vol regime: `Dyn3TV`, `ByiAbN`, `FAT854`, `Fvi4uceM`, `CpqykQ`, `BfVAP`, `6yFA6`, `3gVDne`, `7casjxp`.
2. **High-churn bots** — hundreds–thousands of inc/dec events, near-zero net delta vs gross flow. Likely managed-LP services or proprietary market makers: `KVVrxq` (17,300 T gross flow), `HdHZe1` (3,642 increases), `Evga86`, `9Z6qhmZ` (1,657 events), `EYKWgg` (673 events).

### Caveats
- "Owner" = `position_authority` (signer of inc/dec). Usually equals NFT owner; differs if delegated.
- Net delta is in **raw Whirlpool L units, not USD**. Cross-LP ranking is valid; absolute USD requires per-position tick range + current price (v2 work).
- 30-day window only — long-term whales who didn't act in this window are invisible.

### Observation
The pool is **whale-concentrated**: a single LP holds more than half of the top-50 net liquidity additions in this window. That changes the competitive picture from "big-pool, take-modest-share" to "big-pool, top-1 LP defines the in-range liquidity floor." A small deposit from us would represent a tiny share of in-range liquidity at any given time — likely <0.1% if the dominant LP keeps similar capital deployed.

However: the **active rebalancers** are interesting. `HdHZe1` is rebalancing every ~12 minutes with a position that's #7 by net contribution. That's a managed-LP service in action — and they're profitable enough to keep doing it at scale, which is the strongest possible signal that this pool is genuinely fee-generative for active rebalancers.

## Synthesis

(Filled in Task 6.)
