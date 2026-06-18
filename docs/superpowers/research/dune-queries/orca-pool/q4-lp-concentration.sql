-- Q4: LP concentration for a single Whirlpool pool.
-- Identifies the top liquidity providers ranked by net liquidity contributed
-- (sum of increase_liquidity - sum of decrease_liquidity) over the lookback window.
--
-- Parameters:
--   pool_address (text): the Whirlpool pubkey to filter on.
--   days        (number, default 30): lookback window.
--
-- Notes:
--   - Owner = account_position_authority (signer of the inc/dec call). This is the wallet actively
--     managing the position. It usually equals the position-NFT owner; if delegated they may differ.
--   - Snake_case + camelCase + camelCase_v2 table variants are UNION'd then de-duplicated on
--     (tx_id, outer_idx, inner_idx) to avoid double-counting across IDL eras.
--   - liquidity_amount is uint256 (Whirlpool's L); cast to DOUBLE for aggregation. Precision loss is
--     acceptable for relative-rank ordering.
--   - Net delta is in raw L units, NOT USD. Cross-LP comparison is valid; absolute interpretation
--     would require current price + tick range per position.
WITH inc_raw AS (
  SELECT call_tx_id AS tx_id, call_outer_instruction_index AS o, call_inner_instruction_index AS i,
         account_position_authority AS owner,
         CAST(liquidity_amount AS DOUBLE) AS liq
  FROM whirlpool_solana.whirlpool_call_increase_liquidity
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  SELECT call_tx_id, call_outer_instruction_index, call_inner_instruction_index,
         account_positionAuthority, CAST(liquidityAmount AS DOUBLE)
  FROM whirlpool_solana.whirlpool_call_increaseliquidity
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  SELECT call_tx_id, call_outer_instruction_index, call_inner_instruction_index,
         account_positionAuthority, CAST(liquidityAmount AS DOUBLE)
  FROM whirlpool_solana.whirlpool_call_increaseliquidityv2
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
),
dec_raw AS (
  SELECT call_tx_id AS tx_id, call_outer_instruction_index AS o, call_inner_instruction_index AS i,
         account_position_authority AS owner,
         CAST(liquidity_amount AS DOUBLE) AS liq
  FROM whirlpool_solana.whirlpool_call_decrease_liquidity
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  SELECT call_tx_id, call_outer_instruction_index, call_inner_instruction_index,
         account_positionAuthority, CAST(liquidityAmount AS DOUBLE)
  FROM whirlpool_solana.whirlpool_call_decreaseliquidity
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  SELECT call_tx_id, call_outer_instruction_index, call_inner_instruction_index,
         account_positionAuthority, CAST(liquidityAmount AS DOUBLE)
  FROM whirlpool_solana.whirlpool_call_decreaseliquidityv2
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
),
inc_dedup AS (
  SELECT tx_id, o, i, ARBITRARY(owner) AS owner, ARBITRARY(liq) AS liq
  FROM inc_raw GROUP BY tx_id, o, i
),
dec_dedup AS (
  SELECT tx_id, o, i, ARBITRARY(owner) AS owner, ARBITRARY(liq) AS liq
  FROM dec_raw GROUP BY tx_id, o, i
),
inc_by_owner AS (
  SELECT owner, SUM(liq) AS total_increase, COUNT(*) AS increase_count
  FROM inc_dedup GROUP BY owner
),
dec_by_owner AS (
  SELECT owner, SUM(liq) AS total_decrease, COUNT(*) AS decrease_count
  FROM dec_dedup GROUP BY owner
),
combined AS (
  SELECT
    COALESCE(i.owner, d.owner)        AS owner,
    COALESCE(i.total_increase, 0)     AS total_increase,
    COALESCE(d.total_decrease, 0)     AS total_decrease,
    COALESCE(i.increase_count, 0)     AS increase_count,
    COALESCE(d.decrease_count, 0)     AS decrease_count
  FROM inc_by_owner i
  FULL OUTER JOIN dec_by_owner d ON i.owner = d.owner
)
SELECT
  owner,
  increase_count,
  decrease_count,
  total_increase,
  total_decrease,
  total_increase - total_decrease AS net_liquidity_delta
FROM combined
ORDER BY net_liquidity_delta DESC
LIMIT 50
