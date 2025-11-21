use anyhow::{anyhow, Result};
use borsh::BorshDeserialize;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

pub const RAYDIUM_CLMM_PROGRAM_ID: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";

/// Pubkey form of the Raydium CLMM program ID for owner-verification calls.
pub fn raydium_clmm_program_pubkey() -> Pubkey {
    Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).expect("hardcoded RAYDIUM_CLMM_PROGRAM_ID is valid")
}

const DISC: usize = 8;

/// Key fields from a Raydium CLMM PoolState account.
///
/// IMPORTANT: Verify field order against the actual program source before
/// testing on mainnet. Borsh is order-sensitive — layout-only fields are
/// prefixed with `_` so we keep the on-chain layout without triggering
/// dead-code warnings or needing a struct-wide `#[allow(dead_code)]`.
///
/// Source: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/states/pool.rs
#[derive(BorshDeserialize, Debug, Clone)]
pub struct RaydiumPool {
    pub _bump: [u8; 1],
    pub _amm_config: Pubkey,
    pub _owner: Pubkey,
    pub _token_mint_0: Pubkey,
    pub _token_mint_1: Pubkey,
    pub _token_vault_0: Pubkey,
    pub _token_vault_1: Pubkey,
    pub _observation_key: Pubkey,
    pub _mint_decimals_0: u8,
    pub _mint_decimals_1: u8,
    pub _tick_spacing: u16,
    pub _liquidity: u128,
    pub sqrt_price_x64: u128, // same Q64.64 format as Orca
    pub tick_current: i32,
    // remaining fields omitted (tail of the account).
}

/// Key fields from a Raydium CLMM PersonalPositionState account.
#[derive(BorshDeserialize, Debug, Clone)]
pub struct RaydiumPosition {
    pub _bump: [u8; 1],
    pub _nft_mint: Pubkey,
    pub pool_id: Pubkey,
    pub tick_lower_index: i32,
    pub tick_upper_index: i32,
    pub liquidity: u128,
    pub _fee_growth_inside_0_last_x64: u128,
    pub _fee_growth_inside_1_last_x64: u128,
    pub _token_fees_owed_0: u64,
    pub _token_fees_owed_1: u64,
}

pub fn parse_pool(data: &[u8]) -> Result<RaydiumPool> {
    if data.len() < DISC {
        return Err(anyhow!("Account data too short: {} bytes", data.len()));
    }
    RaydiumPool::try_from_slice(&data[DISC..])
        .map_err(|e| anyhow!("Failed to deserialize Raydium pool: {}", e))
}

pub fn parse_position(data: &[u8]) -> Result<RaydiumPosition> {
    if data.len() < DISC {
        return Err(anyhow!("Account data too short: {} bytes", data.len()));
    }
    RaydiumPosition::try_from_slice(&data[DISC..])
        .map_err(|e| anyhow!("Failed to deserialize Raydium position: {}", e))
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
