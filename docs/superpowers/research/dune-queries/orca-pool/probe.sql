-- Probe: confirm pool exists, surface vaults + token mints, validate filter columns.
-- Parameters:
--   pool_address (text): the Whirlpool pubkey to inspect.
-- Strategy: take 1 recent swap from the pool, join to dex_solana.trades for USD/symbol enrichment.
WITH pool_swap AS (
  SELECT
    call_tx_id,
    call_outer_instruction_index,
    call_inner_instruction_index,
    call_block_time,
    account_whirlpool,
    account_token_vault_a,
    account_token_vault_b
  FROM whirlpool_solana.whirlpool_call_swap
  WHERE account_whirlpool = '{{pool_address}}'
    AND call_block_time >= now() - interval '7' day
  ORDER BY call_block_time ASC
  LIMIT 1
)
SELECT
  ps.account_whirlpool      AS pool_address,
  ps.account_token_vault_a  AS vault_a,
  ps.account_token_vault_b  AS vault_b,
  t.token_bought_mint_address AS token_bought_mint,
  t.token_bought_symbol       AS token_bought_symbol,
  t.token_sold_mint_address   AS token_sold_mint,
  t.token_sold_symbol         AS token_sold_symbol,
  t.fee_tier                AS fee_tier,
  t.amount_usd              AS sample_amount_usd,
  ps.call_block_time        AS first_swap_seen_in_window
FROM pool_swap ps
LEFT JOIN dex_solana.trades t
  ON  t.tx_id = ps.call_tx_id
  AND t.outer_instruction_index = ps.call_outer_instruction_index
  AND t.inner_instruction_index = ps.call_inner_instruction_index
  AND t.block_month >= date_trunc('month', now() - interval '7' day)
  AND t.block_time >= now() - interval '7' day
