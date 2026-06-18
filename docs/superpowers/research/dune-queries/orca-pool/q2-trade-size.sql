-- Q2: Trade-size distribution for a single Whirlpool pool.
-- Returns two result sets via UNION ALL with a discriminator column.
-- Parameters:
--   pool_address (text): the Whirlpool pubkey to filter on.
--   days        (number, default 7): lookback window.
WITH pool_swaps AS (
  SELECT
    call_tx_id                    AS tx_id,
    call_outer_instruction_index  AS outer_instruction_index,
    call_inner_instruction_index  AS inner_instruction_index
  FROM whirlpool_solana.whirlpool_call_swap
  WHERE account_whirlpool = '{{pool_address}}'
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
