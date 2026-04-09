use anyhow::{anyhow, Result};
use borsh::{BorshDeserialize, BorshSerialize};
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
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
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
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
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
    use borsh::BorshSerialize;

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

    /// Serialize a `RaydiumPool` with known values, prepend an 8-byte
    /// discriminator, then deserialize via `parse_pool` and assert every
    /// field round-trips correctly.  This guards against silent field-order
    /// regressions when the struct layout is updated.
    #[test]
    fn raydium_pool_roundtrip() {
        let original = RaydiumPool {
            _bump: [42u8],
            _amm_config: Pubkey::from_str("11111111111111111111111111111111").unwrap(),
            _owner: Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap(),
            _token_mint_0: Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap(),
            _token_mint_1: Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
                .unwrap(),
            _token_vault_0: Pubkey::from_str("11111111111111111111111111111111").unwrap(),
            _token_vault_1: Pubkey::from_str("11111111111111111111111111111111").unwrap(),
            _observation_key: Pubkey::from_str("11111111111111111111111111111111").unwrap(),
            _mint_decimals_0: 9,
            _mint_decimals_1: 6,
            _tick_spacing: 64,
            _liquidity: 123_456_789_u128,
            sqrt_price_x64: 0x0001_0000_0000_0000_0000u128,
            tick_current: -512,
        };

        // Serialize struct body (no discriminator).
        let mut body = Vec::new();
        original.serialize(&mut body).expect("serialize RaydiumPool");

        // Prepend an 8-byte discriminator (arbitrary bytes, not validated by parse_pool).
        let mut data = vec![0xde, 0xad, 0xbe, 0xef, 0x00, 0x01, 0x02, 0x03];
        data.extend_from_slice(&body);

        let parsed = parse_pool(&data).expect("parse_pool should succeed on round-trip data");

        assert_eq!(parsed._bump, original._bump);
        assert_eq!(parsed._amm_config, original._amm_config);
        assert_eq!(parsed._owner, original._owner);
        assert_eq!(parsed._token_mint_0, original._token_mint_0);
        assert_eq!(parsed._token_mint_1, original._token_mint_1);
        assert_eq!(parsed._token_vault_0, original._token_vault_0);
        assert_eq!(parsed._token_vault_1, original._token_vault_1);
        assert_eq!(parsed._observation_key, original._observation_key);
        assert_eq!(parsed._mint_decimals_0, original._mint_decimals_0);
        assert_eq!(parsed._mint_decimals_1, original._mint_decimals_1);
        assert_eq!(parsed._tick_spacing, original._tick_spacing);
        assert_eq!(parsed._liquidity, original._liquidity);
        assert_eq!(parsed.sqrt_price_x64, original.sqrt_price_x64);
        assert_eq!(parsed.tick_current, original.tick_current);
    }

    /// Same round-trip guard for `RaydiumPosition`.
    #[test]
    fn raydium_position_roundtrip() {
        let pool_id = Pubkey::from_str("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK").unwrap();
        let nft_mint = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();

        let original = RaydiumPosition {
            _bump: [7u8],
            _nft_mint: nft_mint,
            pool_id,
            tick_lower_index: -1024,
            tick_upper_index: 1024,
            liquidity: 999_000_000_u128,
            _fee_growth_inside_0_last_x64: 0x0000_1234_5678_u128,
            _fee_growth_inside_1_last_x64: 0x0000_abcd_ef01_u128,
            _token_fees_owed_0: 100_u64,
            _token_fees_owed_1: 200_u64,
        };

        let mut body = Vec::new();
        original.serialize(&mut body).expect("serialize RaydiumPosition");

        let mut data = vec![0u8; 8]; // discriminator placeholder
        data.extend_from_slice(&body);

        let parsed =
            parse_position(&data).expect("parse_position should succeed on round-trip data");

        assert_eq!(parsed._bump, original._bump);
        assert_eq!(parsed._nft_mint, original._nft_mint);
        assert_eq!(parsed.pool_id, original.pool_id);
        assert_eq!(parsed.tick_lower_index, original.tick_lower_index);
        assert_eq!(parsed.tick_upper_index, original.tick_upper_index);
        assert_eq!(parsed.liquidity, original.liquidity);
        assert_eq!(
            parsed._fee_growth_inside_0_last_x64,
            original._fee_growth_inside_0_last_x64
        );
        assert_eq!(
            parsed._fee_growth_inside_1_last_x64,
            original._fee_growth_inside_1_last_x64
        );
        assert_eq!(parsed._token_fees_owed_0, original._token_fees_owed_0);
        assert_eq!(parsed._token_fees_owed_1, original._token_fees_owed_1);
    }
}
