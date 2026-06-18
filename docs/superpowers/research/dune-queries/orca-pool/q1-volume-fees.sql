-- Q1: Daily volume + fee summary for a single Whirlpool pool.
-- Parameters:
--   pool_address (text): the Whirlpool pubkey to filter on.
--   days        (number): lookback window in days.
-- Output: one row per UTC day with swap_count, volume_usd, fee_usd, distinct_traders,
--         mean_trade_usd, median_trade_usd, fee_tier.
WITH pool_swaps AS (
  SELECT
    call_tx_id                    AS tx_id,
    call_outer_instruction_index  AS outer_instruction_index,
    call_inner_instruction_index  AS inner_instruction_index,
    call_block_time               AS block_time
  FROM whirlpool_solana.whirlpool_call_swap
  WHERE account_whirlpool = '{{pool_address}}'
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
  COUNT(*)                            AS swap_count,
  SUM(amount_usd)                     AS volume_usd,
  SUM(fee_usd)                        AS fee_usd,
  COUNT(DISTINCT trader_id)           AS distinct_traders,
  AVG(amount_usd)                     AS mean_trade_usd,
  approx_percentile(amount_usd, 0.5)  AS median_trade_usd,
  ARBITRARY(fee_tier)                 AS fee_tier
FROM enriched
GROUP BY 1
ORDER BY 1 ASC
