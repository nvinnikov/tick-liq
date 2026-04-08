use anyhow::{anyhow, Result};
use borsh::BorshDeserialize;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

pub const WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

/// Pubkey form of the Whirlpool program ID for owner-verification calls.
/// Panics only on a developer error (the constant above is malformed) — never
/// on user input.
pub fn whirlpool_program_pubkey() -> Pubkey {
    Pubkey::from_str(WHIRLPOOL_PROGRAM_ID).expect("hardcoded WHIRLPOOL_PROGRAM_ID is valid")
}

/// First 8 bytes of every Anchor account are a discriminator — skip them.
const DISC: usize = 8;

/// Key fields of an Orca Whirlpool pool account.
///
/// Field order MUST match the on-chain Anchor struct exactly — borsh is
/// position-sensitive, so we cannot omit "unused" fields without corrupting
/// every field that follows. Layout-only fields are prefixed with `_` so
/// the dead-code lint understands they exist solely to satisfy borsh, and
/// no broad `#[allow(dead_code)]` suppression is required.
///
/// Reference: https://github.com/orca-so/whirlpools/blob/main/programs/whirlpool/src/state/whirlpool.rs
#[derive(BorshDeserialize, Debug, Clone)]
pub struct WhirlpoolPool {
    pub _whirlpools_config: Pubkey,
    pub _whirlpool_bump: [u8; 1],
    pub _tick_spacing: u16,
    pub _tick_spacing_seed: [u8; 2],
    pub fee_rate: u16, // hundredths of a bip; 300 = 0.03%
    pub _protocol_fee_rate: u16,
    pub liquidity: u128,
    pub sqrt_price: u128, // Q64.64 fixed-point
    pub tick_current_index: i32,
    pub _protocol_fee_owed_a: u64,
    pub _protocol_fee_owed_b: u64,
    pub _token_mint_a: Pubkey,
    pub _token_vault_a: Pubkey,
    pub fee_growth_global_a: u128,
    pub _token_mint_b: Pubkey,
    pub _token_vault_b: Pubkey,
    pub fee_growth_global_b: u128,
    pub _reward_last_updated_timestamp: u64,
    // reward_infos (3 × 128 bytes) omitted — they sit at the tail so dropping
    // them does not affect any preceding field offsets.
}

/// Key fields of an Orca Whirlpool position account.
#[derive(BorshDeserialize, Debug, Clone)]
pub struct WhirlpoolPosition {
    pub whirlpool: Pubkey,
    pub _position_mint: Pubkey,
    pub liquidity: u128,
    pub tick_lower_index: i32,
    pub tick_upper_index: i32,
    pub fee_growth_checkpoint_a: u128,
    pub fee_owed_a: u64,
    pub fee_growth_checkpoint_b: u128,
    pub fee_owed_b: u64,
    // reward_infos omitted (tail fields).
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
