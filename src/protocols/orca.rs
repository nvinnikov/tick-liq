use anyhow::{anyhow, Result};
use borsh::BorshDeserialize;
use solana_sdk::pubkey::Pubkey;

pub const WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

/// First 8 bytes of every Anchor account are a discriminator — skip them.
const DISC: usize = 8;

/// Key fields of an Orca Whirlpool pool account.
/// Field order matches the on-chain Anchor struct exactly.
/// Reference: https://github.com/orca-so/whirlpools/blob/main/programs/whirlpool/src/state/whirlpool.rs
#[derive(BorshDeserialize, Debug, Clone)]
pub struct WhirlpoolPool {
    pub whirlpools_config: Pubkey,
    pub whirlpool_bump: [u8; 1],
    pub tick_spacing: u16,
    pub tick_spacing_seed: [u8; 2],
    pub fee_rate: u16,           // hundredths of a bip; 300 = 0.03%
    pub protocol_fee_rate: u16,
    pub liquidity: u128,
    pub sqrt_price: u128,        // Q64.64 fixed-point
    pub tick_current_index: i32,
    pub protocol_fee_owed_a: u64,
    pub protocol_fee_owed_b: u64,
    pub token_mint_a: Pubkey,
    pub token_vault_a: Pubkey,
    pub fee_growth_global_a: u128,
    pub token_mint_b: Pubkey,
    pub token_vault_b: Pubkey,
    pub fee_growth_global_b: u128,
    pub reward_last_updated_timestamp: u64,
    // reward_infos (3 × 128 bytes) omitted — not needed for analytics
}

/// Key fields of an Orca Whirlpool position account.
#[derive(BorshDeserialize, Debug, Clone)]
pub struct WhirlpoolPosition {
    pub whirlpool: Pubkey,
    pub position_mint: Pubkey,
    pub liquidity: u128,
    pub tick_lower_index: i32,
    pub tick_upper_index: i32,
    pub fee_growth_checkpoint_a: u128,
    pub fee_owed_a: u64,
    pub fee_growth_checkpoint_b: u128,
    pub fee_owed_b: u64,
    // reward_infos omitted
}

pub fn parse_pool(data: &[u8]) -> Result<WhirlpoolPool> {
    if data.len() < DISC {
        return Err(anyhow!("Account data too short: {} bytes", data.len()));
    }
    WhirlpoolPool::try_from_slice(&data[DISC..])
        .map_err(|e| anyhow!("Failed to deserialize Whirlpool pool: {}", e))
}

pub fn parse_position(data: &[u8]) -> Result<WhirlpoolPosition> {
    if data.len() < DISC {
        return Err(anyhow!("Account data too short: {} bytes", data.len()));
    }
    WhirlpoolPosition::try_from_slice(&data[DISC..])
        .map_err(|e| anyhow!("Failed to deserialize Whirlpool position: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pool_too_short_returns_error() {
        let result = parse_pool(&[0u8; 4]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too short"));
    }

    #[test]
    fn test_parse_position_too_short_returns_error() {
        let result = parse_position(&[0u8; 4]);
        assert!(result.is_err());
    }
}
