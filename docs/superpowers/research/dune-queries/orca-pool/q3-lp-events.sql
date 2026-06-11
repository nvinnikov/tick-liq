-- Q3: Daily LP event activity for a single Whirlpool pool.
-- Parameters:
--   pool_address (text): the Whirlpool pubkey to filter on.
--   days        (number, default 30): lookback window.
-- Notes:
--   - Dune indexes Whirlpool decoded calls under TWO naming conventions (snake_case and camelCase)
--     because the Anchor IDL was changed mid-life. We UNION ALL both then DISTINCT by
--     (tx_id, outer_idx, inner_idx, kind) to avoid double-counting.
--   - whirlpool_call_close_position has no account_whirlpool arg (close doesn't reference the pool).
--     We use decrease_liquidity as a close-activity proxy for v1.
--   - whirlpool_call_increaseliquidityv2 is the v2 (TokenExtensions) variant; included.
WITH all_events_raw AS (
  -- open_position (snake)
  SELECT call_tx_id AS tx_id, call_outer_instruction_index AS o, call_inner_instruction_index AS i,
         date_trunc('day', call_block_time) AS day, 'open' AS kind
  FROM whirlpool_solana.whirlpool_call_open_position
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  -- open_position (camel)
  SELECT call_tx_id, call_outer_instruction_index, call_inner_instruction_index,
         date_trunc('day', call_block_time), 'open'
  FROM whirlpool_solana.whirlpool_call_openposition
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  -- increase_liquidity (snake)
  SELECT call_tx_id, call_outer_instruction_index, call_inner_instruction_index,
         date_trunc('day', call_block_time), 'increase'
  FROM whirlpool_solana.whirlpool_call_increase_liquidity
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  -- increase_liquidity (camel)
  SELECT call_tx_id, call_outer_instruction_index, call_inner_instruction_index,
         date_trunc('day', call_block_time), 'increase'
  FROM whirlpool_solana.whirlpool_call_increaseliquidity
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  -- increase_liquidity v2 (camel)
  SELECT call_tx_id, call_outer_instruction_index, call_inner_instruction_index,
         date_trunc('day', call_block_time), 'increase'
  FROM whirlpool_solana.whirlpool_call_increaseliquidityv2
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  -- decrease_liquidity (snake)
  SELECT call_tx_id, call_outer_instruction_index, call_inner_instruction_index,
         date_trunc('day', call_block_time), 'decrease'
  FROM whirlpool_solana.whirlpool_call_decrease_liquidity
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  -- decrease_liquidity (camel)
  SELECT call_tx_id, call_outer_instruction_index, call_inner_instruction_index,
         date_trunc('day', call_block_time), 'decrease'
  FROM whirlpool_solana.whirlpool_call_decreaseliquidity
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
  UNION ALL
  -- decrease_liquidity v2 (camel)
  SELECT call_tx_id, call_outer_instruction_index, call_inner_instruction_index,
         date_trunc('day', call_block_time), 'decrease'
  FROM whirlpool_solana.whirlpool_call_decreaseliquidityv2
  WHERE account_whirlpool = '{{pool_address}}' AND call_block_time >= now() - interval '{{days}}' day
),
deduped AS (
  SELECT DISTINCT tx_id, o, i, day, kind FROM all_events_raw
)
SELECT
  day,
  SUM(CASE WHEN kind = 'open'     THEN 1 ELSE 0 END) AS positions_opened,
  SUM(CASE WHEN kind = 'increase' THEN 1 ELSE 0 END) AS increase_liquidity,
  SUM(CASE WHEN kind = 'decrease' THEN 1 ELSE 0 END) AS decrease_liquidity
FROM deduped
GROUP BY day
ORDER BY day ASC
