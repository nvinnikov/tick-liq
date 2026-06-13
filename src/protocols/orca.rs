use crate::rpc::SolanaRpc;
use anyhow::{Result, anyhow};
use borsh::BorshDeserialize;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

/// Whirlpool TickArray size (number of ticks per account).
pub const TICK_ARRAY_SIZE: usize = 88;
/// Whirlpool reward slots.
pub const NUM_REWARDS: usize = 3;

pub const WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

/// Pubkey form of the Whirlpool program ID for owner-verification calls.
/// Panics only on a developer error (the constant above is malformed) — never
/// on user input.
pub fn whirlpool_program_pubkey() -> Pubkey {
    Pubkey::from_str(WHIRLPOOL_PROGRAM_ID).expect("hardcoded WHIRLPOOL_PROGRAM_ID is valid")
}

/// First 8 bytes of every Anchor account are a discriminator — skip them.
const DISC: usize = 8;

// Canonical Anchor account discriminators for the Whirlpool program
// (sha256("account:<Name>")[..8], from the published IDL). We use these
// defensively: a parser rejects data that is positively a *different* known
// account type (e.g. a TickArray address passed as `--pool`), which the owner
// check alone cannot catch since all three are owned by the same program.
// Matching against the other types (rather than requiring an exact self-match)
// means a stale constant can only fail to catch a misuse, never reject a
// genuine account.
const WHIRLPOOL_DISC: [u8; 8] = [63, 149, 209, 12, 225, 128, 99, 9];
const POSITION_DISC: [u8; 8] = [170, 188, 143, 228, 122, 64, 247, 208];
const TICK_ARRAY_DISC: [u8; 8] = [69, 97, 189, 190, 110, 7, 66, 187];

/// Reject `data` if its discriminator matches one of `forbidden` (a list of
/// other known account types). `what` names the type we were trying to parse.
fn reject_wrong_account_type(data: &[u8], forbidden: &[(&str, [u8; 8])], what: &str) -> Result<()> {
    if data.len() < DISC {
        return Ok(()); // length is checked by the caller
    }
    let disc = &data[..DISC];
    for (name, bytes) in forbidden {
        if disc == bytes {
            return Err(anyhow!(
                "expected a Whirlpool {} account but got a {} account (wrong address?)",
                what,
                name
            ));
        }
    }
    Ok(())
}

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
    pub tick_spacing: u16,
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
    pub _fee_growth_checkpoint_a: u128,
    pub fee_owed_a: u64,
    pub _fee_growth_checkpoint_b: u128,
    pub fee_owed_b: u64,
    // reward_infos omitted (tail fields).
}

/// A single tick slot inside a Whirlpool TickArray account.
///
/// Layout matches the Anchor on-chain struct; unused fields are prefixed with
/// `_` to satisfy borsh positional deserialization without a blanket dead-code
/// allow.
#[derive(BorshDeserialize, Debug, Clone)]
pub struct Tick {
    pub initialized: bool,
    pub liquidity_net: i128,
    pub _liquidity_gross: u128,
    pub _fee_growth_outside_a: u128,
    pub _fee_growth_outside_b: u128,
    pub _reward_growths_outside: [u128; NUM_REWARDS],
}

/// A Whirlpool TickArray account holding `TICK_ARRAY_SIZE` ticks.
#[derive(BorshDeserialize, Debug, Clone)]
pub struct TickArray {
    pub start_tick_index: i32,
    pub ticks: [Tick; TICK_ARRAY_SIZE],
    pub _whirlpool: Pubkey,
}

pub fn parse_tick_array(data: &[u8]) -> Result<TickArray> {
    if data.len() < DISC {
        return Err(anyhow!("TickArray account too short: {} bytes", data.len()));
    }
    TickArray::try_from_slice(&data[DISC..])
        .map_err(|e| anyhow!("Failed to deserialize TickArray: {}", e))
}

/// Derive the TickArray PDA for a given pool + start tick.
///
/// Seeds: [b"tick_array", whirlpool, start_tick_index.to_string().as_bytes()]
pub fn tick_array_pda(whirlpool: &Pubkey, start_tick_index: i32) -> Pubkey {
    let start_str = start_tick_index.to_string();
    let (pda, _) = Pubkey::find_program_address(
        &[b"tick_array", whirlpool.as_ref(), start_str.as_bytes()],
        &whirlpool_program_pubkey(),
    );
    pda
}

/// Floor `current_tick` to the nearest start-tick-index for a tick array with
/// the given `tick_spacing`. Handles negative ticks via Euclidean division.
pub fn tick_array_start_index(current_tick: i32, tick_spacing: u16) -> i32 {
    let span = tick_spacing as i32 * TICK_ARRAY_SIZE as i32;
    current_tick.div_euclid(span) * span
}

/// Fetch up to 5 TickArray accounts centered on `current_tick` (2 below, the
/// current one, and 2 above). Missing or unparseable arrays are skipped with a
/// warning so one cold array doesn't poison the whole depth view.
pub fn fetch_tick_arrays(
    rpc: &SolanaRpc,
    whirlpool: &Pubkey,
    current_tick: i32,
    tick_spacing: u16,
) -> Result<Vec<TickArray>> {
    let program = whirlpool_program_pubkey();
    let span = tick_spacing as i32 * TICK_ARRAY_SIZE as i32;
    let center_start = tick_array_start_index(current_tick, tick_spacing);

    // Derive all 5 PDAs, then fetch them in ONE round-trip. The previous
    // per-array fetch paid the full 3-attempt retry/backoff for every cold
    // (uninitialized) array — up to ~3.75s of pure sleep for a 5-array miss.
    let starts: Vec<i32> = (-2i32..=2i32).map(|o| center_start + o * span).collect();
    let pdas: Vec<Pubkey> = starts
        .iter()
        .map(|&s| tick_array_pda(whirlpool, s))
        .collect();

    let datas = rpc.get_multiple_accounts_checked(&pdas, &program)?;

    let mut out = Vec::with_capacity(5);
    for (start, data) in starts.iter().zip(datas) {
        match data {
            Some(bytes) => match parse_tick_array(&bytes) {
                Ok(ta) => out.push(ta),
                Err(e) => tracing::warn!("Skipping tick array at start {}: {}", start, e),
            },
            None => tracing::debug!("Tick array at start {} not initialized -- skipping", start),
        }
    }
    Ok(out)
}

pub fn parse_pool(data: &[u8]) -> Result<WhirlpoolPool> {
    if data.len() < DISC {
        return Err(anyhow!("Account data too short: {} bytes", data.len()));
    }
    reject_wrong_account_type(
        data,
        &[("Position", POSITION_DISC), ("TickArray", TICK_ARRAY_DISC)],
        "pool",
    )?;
    let mut cursor = &data[DISC..];
    WhirlpoolPool::deserialize(&mut cursor)
        .map_err(|e| anyhow!("Failed to deserialize Whirlpool pool: {}", e))
}

pub fn parse_position(data: &[u8]) -> Result<WhirlpoolPosition> {
    if data.len() < DISC {
        return Err(anyhow!("Account data too short: {} bytes", data.len()));
    }
    reject_wrong_account_type(
        data,
        &[
            ("Whirlpool", WHIRLPOOL_DISC),
            ("TickArray", TICK_ARRAY_DISC),
        ],
        "position",
    )?;
    let mut cursor = &data[DISC..];
    WhirlpoolPosition::deserialize(&mut cursor)
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

    #[test]
    fn parse_pool_rejects_tick_array_account() {
        // A TickArray address passed as --pool: owner check would pass (same
        // program), but the discriminator marks it as a different type.
        let mut data = vec![0u8; 256];
        data[..8].copy_from_slice(&TICK_ARRAY_DISC);
        let err = parse_pool(&data).unwrap_err().to_string();
        assert!(err.contains("TickArray"), "got: {err}");
    }

    #[test]
    fn parse_position_rejects_whirlpool_account() {
        let mut data = vec![0u8; 256];
        data[..8].copy_from_slice(&WHIRLPOOL_DISC);
        let err = parse_position(&data).unwrap_err().to_string();
        assert!(err.contains("Whirlpool"), "got: {err}");
    }
}
