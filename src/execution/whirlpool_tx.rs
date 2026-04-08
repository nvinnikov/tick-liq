//! Orca Whirlpool instruction builder behind the
//! [`crate::execution::rebalance::TxSubmitter`] trait.
//!
//! ## Why hand-rolled
//!
//! We deliberately do **not** depend on `orca-whirlpools-client` /
//! `anchor-client` for instruction construction. The Anchor instruction
//! wire format is trivial — an 8-byte method discriminator followed by
//! borsh-encoded args, with a fixed account ordering — and pulling
//! `anchor-client` would drag IDL trees and version-pinning friction
//! into a code path that signs transactions. Hand-rolling keeps the
//! supply-chain surface here as thin as the borsh decoders in
//! `protocols/orca.rs`, which is the same trade-off we already made for
//! the read side.
//!
//! Every discriminator constant in [`disc`] is the first 8 bytes of
//! `sha256("global:<method>")` — Anchor's canonical scheme — and is
//! cross-checked at test time against an in-test sha256 computation
//! using `solana_sdk::hash::hash`, which is the same SHA-256 the chain
//! uses. The constants and the methods they came from are listed below
//! so a future reader can re-derive them in seconds:
//!
//! ```text
//! global:close_position           → [123, 134,  81,   0,  49,  68,  98,  98]
//! global:collect_fees             → [164, 152, 207,  99,  30, 186,  19, 182]
//! global:collect_reward           → [ 70,   5, 132,  87,  86, 235, 177,  34]
//! global:update_fees_and_rewards  → [154, 230, 250,  13, 236, 209,  75, 223]
//! ```
//!
//! Account orderings are taken from the on-chain Whirlpool program at
//! the same revision as `protocols/orca.rs`'s account decoders:
//! <https://github.com/orca-so/whirlpools/tree/main/programs/whirlpool/src/instructions>.
//!
//! ## Scope of this PR (#31a)
//!
//! Implements `close_position`, `collect_fees`, and `collect_reward`
//! instruction builders, plus a [`WhirlpoolTxSubmitter`] that wires
//! them together for the *close-side* of a rebalance. The open side
//! (`open_position`, `increase_liquidity`, tick-array PDA derivation,
//! and the full close → collect → open `Vec<Instruction>` returned to
//! `TxSigner`) is the next PR (#31b). Until then, calling
//! `submit_rebalance` returns a clear `bail!` describing the missing
//! piece — **never** `unimplemented!()`, because that would panic in
//! production if the orchestration engine ever ran this submitter
//! before #31b lands.
//!
//! Snapshot tests pin the discriminator + borsh layout + account
//! ordering for each builder against fixtures so a refactor cannot
//! silently change the wire bytes the program would see.

use anyhow::{bail, Result};
use async_trait::async_trait;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;

use crate::execution::rebalance::{RebalancePlan, TxSubmitter};

/// Orca Whirlpool program id on Solana mainnet-beta.
/// Source: <https://docs.orca.so/whirlpools/architecture/program>.
pub const WHIRLPOOL_PROGRAM_ID_STR: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

/// Anchor method discriminators for the Whirlpool instructions we use.
///
/// Each is the first 8 bytes of `sha256("global:<method>")`. See the
/// `discriminators_match_anchor_scheme` test for an in-process re-derivation.
pub mod disc {
    /// `global:close_position`
    pub const CLOSE_POSITION: [u8; 8] = [123, 134, 81, 0, 49, 68, 98, 98];
    /// `global:collect_fees`
    pub const COLLECT_FEES: [u8; 8] = [164, 152, 207, 99, 30, 186, 19, 182];
    /// `global:collect_reward`
    pub const COLLECT_REWARD: [u8; 8] = [70, 5, 132, 87, 86, 235, 177, 34];
    /// `global:update_fees_and_rewards`
    pub const UPDATE_FEES_AND_REWARDS: [u8; 8] = [154, 230, 250, 13, 236, 209, 75, 223];
}

/// Account inputs for [`build_close_position_ix`].
///
/// Account ordering matches the on-chain `ClosePosition` accounts
/// struct: position_authority (signer), receiver (sol rent dest),
/// position pda, position_mint, position_token_account, token_program.
#[derive(Debug, Clone, Copy)]
pub struct CloseAccounts {
    pub position_authority: Pubkey,
    pub receiver: Pubkey,
    pub position: Pubkey,
    pub position_mint: Pubkey,
    pub position_token_account: Pubkey,
    pub token_program: Pubkey,
}

/// Build a Whirlpool `close_position` instruction.
///
/// `close_position` itself takes no Anchor args — the on-chain handler
/// just verifies that liquidity == 0 and burns/closes the NFT. The
/// instruction data is therefore exactly the 8-byte discriminator.
pub fn build_close_position_ix(program_id: &Pubkey, accs: CloseAccounts) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(accs.position_authority, true),
            AccountMeta::new(accs.receiver, false),
            AccountMeta::new(accs.position, false),
            AccountMeta::new(accs.position_mint, false),
            AccountMeta::new(accs.position_token_account, false),
            AccountMeta::new_readonly(accs.token_program, false),
        ],
        data: disc::CLOSE_POSITION.to_vec(),
    }
}

/// Account inputs for [`build_collect_fees_ix`].
///
/// Matches the `CollectFees` accounts struct: whirlpool, position_authority
/// (signer), position pda, position_token_account, token_owner_account_a,
/// token_vault_a, token_owner_account_b, token_vault_b, token_program.
#[derive(Debug, Clone, Copy)]
pub struct CollectFeesAccounts {
    pub whirlpool: Pubkey,
    pub position_authority: Pubkey,
    pub position: Pubkey,
    pub position_token_account: Pubkey,
    pub token_owner_account_a: Pubkey,
    pub token_vault_a: Pubkey,
    pub token_owner_account_b: Pubkey,
    pub token_vault_b: Pubkey,
    pub token_program: Pubkey,
}

/// Build a Whirlpool `collect_fees` instruction. No Anchor args.
pub fn build_collect_fees_ix(program_id: &Pubkey, accs: CollectFeesAccounts) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(accs.whirlpool, false),
            AccountMeta::new_readonly(accs.position_authority, true),
            AccountMeta::new(accs.position, false),
            AccountMeta::new_readonly(accs.position_token_account, false),
            AccountMeta::new(accs.token_owner_account_a, false),
            AccountMeta::new(accs.token_vault_a, false),
            AccountMeta::new(accs.token_owner_account_b, false),
            AccountMeta::new(accs.token_vault_b, false),
            AccountMeta::new_readonly(accs.token_program, false),
        ],
        data: disc::COLLECT_FEES.to_vec(),
    }
}

/// Account inputs for [`build_collect_reward_ix`].
///
/// Matches the `CollectReward` accounts struct: whirlpool, position_authority
/// (signer), position pda, position_token_account, reward_owner_account,
/// reward_vault, token_program.
#[derive(Debug, Clone, Copy)]
pub struct CollectRewardAccounts {
    pub whirlpool: Pubkey,
    pub position_authority: Pubkey,
    pub position: Pubkey,
    pub position_token_account: Pubkey,
    pub reward_owner_account: Pubkey,
    pub reward_vault: Pubkey,
    pub token_program: Pubkey,
}

/// Build a Whirlpool `collect_reward` instruction.
///
/// The single Anchor arg is `reward_index: u8` — selecting which of
/// the (up to 3) reward slots on the pool to collect from.
pub fn build_collect_reward_ix(
    program_id: &Pubkey,
    accs: CollectRewardAccounts,
    reward_index: u8,
) -> Instruction {
    let mut data = Vec::with_capacity(disc::COLLECT_REWARD.len() + 1);
    data.extend_from_slice(&disc::COLLECT_REWARD);
    data.push(reward_index);
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(accs.whirlpool, false),
            AccountMeta::new_readonly(accs.position_authority, true),
            AccountMeta::new_readonly(accs.position, false),
            AccountMeta::new_readonly(accs.position_token_account, false),
            AccountMeta::new(accs.reward_owner_account, false),
            AccountMeta::new(accs.reward_vault, false),
            AccountMeta::new_readonly(accs.token_program, false),
        ],
        data,
    }
}

/// Account inputs for [`build_update_fees_and_rewards_ix`].
///
/// `update_fees_and_rewards` is the Anchor handler that ticks the
/// position's accumulators forward to the current pool state — Orca's
/// SDK calls it before `collect_fees` to ensure the on-chain
/// `fee_owed_*` fields reflect the latest fee growth. We expose it
/// because a real rebalance close-side typically does:
///   `update_fees_and_rewards → collect_fees → collect_reward* →
///    decrease_liquidity → close_position`.
/// `decrease_liquidity` is part of #31b (it shares tick-array PDA
/// derivation with `increase_liquidity`).
#[derive(Debug, Clone, Copy)]
pub struct UpdateFeesAndRewardsAccounts {
    pub whirlpool: Pubkey,
    pub position: Pubkey,
    pub tick_array_lower: Pubkey,
    pub tick_array_upper: Pubkey,
}

pub fn build_update_fees_and_rewards_ix(
    program_id: &Pubkey,
    accs: UpdateFeesAndRewardsAccounts,
) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(accs.whirlpool, false),
            AccountMeta::new(accs.position, false),
            AccountMeta::new_readonly(accs.tick_array_lower, false),
            AccountMeta::new_readonly(accs.tick_array_upper, false),
        ],
        data: disc::UPDATE_FEES_AND_REWARDS.to_vec(),
    }
}

// -----------------------------------------------------------------------------
// TxSubmitter wiring
// -----------------------------------------------------------------------------

/// Concrete `TxSubmitter` impl that builds Whirlpool rebalance
/// instructions.
///
/// In #31a this is a stub that errors on `submit_rebalance`: the
/// builder helpers above are usable in isolation (and tested), but the
/// full close → collect → open flow needs the open-side instructions
/// (#31b) and the live account resolver (which knows the position's
/// vaults, the user's ATAs, and the tick array PDAs for the new
/// range). We choose `bail!` over `unimplemented!()` so a misconfigured
/// engine in production raises a normal anyhow error rather than
/// panicking the worker thread.
pub struct WhirlpoolTxSubmitter {
    pub program_id: Pubkey,
}

impl WhirlpoolTxSubmitter {
    pub fn new(program_id: Pubkey) -> Self {
        Self { program_id }
    }

    /// Convenience constructor using [`WHIRLPOOL_PROGRAM_ID_STR`].
    pub fn mainnet() -> Result<Self> {
        let program_id: Pubkey = WHIRLPOOL_PROGRAM_ID_STR
            .parse()
            .map_err(|e| anyhow::anyhow!("parse WHIRLPOOL_PROGRAM_ID_STR: {e}"))?;
        Ok(Self { program_id })
    }
}

#[async_trait]
impl TxSubmitter for WhirlpoolTxSubmitter {
    async fn submit_rebalance(&self, _plan: &RebalancePlan) -> Result<String> {
        bail!(
            "WhirlpoolTxSubmitter::submit_rebalance not yet implemented — \
             open-side instructions and account resolution land in #31b"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::hash::hash as sha256;

    fn anchor_disc(method: &str) -> [u8; 8] {
        let preimage = format!("global:{method}");
        let h = sha256(preimage.as_bytes());
        let bytes = h.to_bytes();
        let mut out = [0u8; 8];
        out.copy_from_slice(&bytes[..8]);
        out
    }

    /// Cross-check every constant in `disc` against an in-process
    /// recomputation of `sha256("global:<method>")[..8]`. If this test
    /// breaks, either the constants drifted from Anchor's scheme or
    /// `solana_sdk::hash::hash` changed under us — both are red flags
    /// worth investigating before shipping bytes to mainnet.
    #[test]
    fn discriminators_match_anchor_scheme() {
        assert_eq!(disc::CLOSE_POSITION, anchor_disc("close_position"));
        assert_eq!(disc::COLLECT_FEES, anchor_disc("collect_fees"));
        assert_eq!(disc::COLLECT_REWARD, anchor_disc("collect_reward"));
        assert_eq!(
            disc::UPDATE_FEES_AND_REWARDS,
            anchor_disc("update_fees_and_rewards")
        );
    }

    /// Pin the program id literal so the constant cannot drift to a
    /// devnet/testnet/wrong-program id without a test break.
    #[test]
    fn whirlpool_program_id_parses_and_is_canonical() {
        let pk: Pubkey = WHIRLPOOL_PROGRAM_ID_STR.parse().unwrap();
        assert_eq!(pk.to_string(), WHIRLPOOL_PROGRAM_ID_STR);
    }

    fn fake_close_accounts() -> CloseAccounts {
        CloseAccounts {
            position_authority: Pubkey::new_from_array([1u8; 32]),
            receiver: Pubkey::new_from_array([2u8; 32]),
            position: Pubkey::new_from_array([3u8; 32]),
            position_mint: Pubkey::new_from_array([4u8; 32]),
            position_token_account: Pubkey::new_from_array([5u8; 32]),
            token_program: Pubkey::new_from_array([6u8; 32]),
        }
    }

    /// Snapshot the close_position instruction layout: program id,
    /// account count, signer/writable flags in order, and full data
    /// bytes. A drift in any of these would change what the on-chain
    /// program sees, so we lock all of them at once.
    #[test]
    fn close_position_ix_snapshot() {
        let program_id = Pubkey::new_from_array([9u8; 32]);
        let ix = build_close_position_ix(&program_id, fake_close_accounts());

        assert_eq!(ix.program_id, program_id);
        assert_eq!(ix.data, disc::CLOSE_POSITION.to_vec());
        assert_eq!(ix.accounts.len(), 6);

        // (is_signer, is_writable) per account, in declaration order.
        let flags: Vec<(bool, bool)> = ix
            .accounts
            .iter()
            .map(|a| (a.is_signer, a.is_writable))
            .collect();
        assert_eq!(
            flags,
            vec![
                (true, false),  // position_authority
                (false, true),  // receiver
                (false, true),  // position
                (false, true),  // position_mint
                (false, true),  // position_token_account
                (false, false), // token_program
            ]
        );

        let pks: Vec<Pubkey> = ix.accounts.iter().map(|a| a.pubkey).collect();
        assert_eq!(
            pks,
            vec![
                Pubkey::new_from_array([1u8; 32]),
                Pubkey::new_from_array([2u8; 32]),
                Pubkey::new_from_array([3u8; 32]),
                Pubkey::new_from_array([4u8; 32]),
                Pubkey::new_from_array([5u8; 32]),
                Pubkey::new_from_array([6u8; 32]),
            ]
        );
    }

    fn fake_collect_fees_accounts() -> CollectFeesAccounts {
        CollectFeesAccounts {
            whirlpool: Pubkey::new_from_array([10u8; 32]),
            position_authority: Pubkey::new_from_array([11u8; 32]),
            position: Pubkey::new_from_array([12u8; 32]),
            position_token_account: Pubkey::new_from_array([13u8; 32]),
            token_owner_account_a: Pubkey::new_from_array([14u8; 32]),
            token_vault_a: Pubkey::new_from_array([15u8; 32]),
            token_owner_account_b: Pubkey::new_from_array([16u8; 32]),
            token_vault_b: Pubkey::new_from_array([17u8; 32]),
            token_program: Pubkey::new_from_array([18u8; 32]),
        }
    }

    #[test]
    fn collect_fees_ix_snapshot() {
        let program_id = Pubkey::new_from_array([99u8; 32]);
        let ix = build_collect_fees_ix(&program_id, fake_collect_fees_accounts());

        assert_eq!(ix.program_id, program_id);
        assert_eq!(ix.data, disc::COLLECT_FEES.to_vec());
        assert_eq!(ix.accounts.len(), 9);

        let flags: Vec<(bool, bool)> = ix
            .accounts
            .iter()
            .map(|a| (a.is_signer, a.is_writable))
            .collect();
        assert_eq!(
            flags,
            vec![
                (false, false), // whirlpool (read-only)
                (true, false),  // position_authority signer
                (false, true),  // position
                (false, false), // position_token_account read-only
                (false, true),  // token_owner_account_a
                (false, true),  // token_vault_a
                (false, true),  // token_owner_account_b
                (false, true),  // token_vault_b
                (false, false), // token_program
            ]
        );
    }

    fn fake_collect_reward_accounts() -> CollectRewardAccounts {
        CollectRewardAccounts {
            whirlpool: Pubkey::new_from_array([20u8; 32]),
            position_authority: Pubkey::new_from_array([21u8; 32]),
            position: Pubkey::new_from_array([22u8; 32]),
            position_token_account: Pubkey::new_from_array([23u8; 32]),
            reward_owner_account: Pubkey::new_from_array([24u8; 32]),
            reward_vault: Pubkey::new_from_array([25u8; 32]),
            token_program: Pubkey::new_from_array([26u8; 32]),
        }
    }

    #[test]
    fn collect_reward_ix_snapshot_encodes_index() {
        let program_id = Pubkey::new_from_array([77u8; 32]);
        let ix = build_collect_reward_ix(&program_id, fake_collect_reward_accounts(), 2);

        assert_eq!(ix.program_id, program_id);
        // Discriminator + 1 byte for reward_index = 9.
        let mut expected = disc::COLLECT_REWARD.to_vec();
        expected.push(2);
        assert_eq!(ix.data, expected);
        assert_eq!(ix.accounts.len(), 7);

        let flags: Vec<(bool, bool)> = ix
            .accounts
            .iter()
            .map(|a| (a.is_signer, a.is_writable))
            .collect();
        assert_eq!(
            flags,
            vec![
                (false, false), // whirlpool
                (true, false),  // position_authority
                (false, false), // position
                (false, false), // position_token_account
                (false, true),  // reward_owner_account
                (false, true),  // reward_vault
                (false, false), // token_program
            ]
        );
    }

    #[test]
    fn collect_reward_index_round_trips_each_slot() {
        // Whirlpool supports up to 3 reward slots; verify the byte
        // encoding for each so we can't accidentally truncate.
        let program_id = Pubkey::new_from_array([1u8; 32]);
        for idx in 0u8..3 {
            let ix = build_collect_reward_ix(&program_id, fake_collect_reward_accounts(), idx);
            assert_eq!(ix.data.last().copied(), Some(idx));
            assert_eq!(ix.data.len(), 9);
        }
    }

    #[test]
    fn update_fees_and_rewards_ix_snapshot() {
        let program_id = Pubkey::new_from_array([55u8; 32]);
        let ix = build_update_fees_and_rewards_ix(
            &program_id,
            UpdateFeesAndRewardsAccounts {
                whirlpool: Pubkey::new_from_array([30u8; 32]),
                position: Pubkey::new_from_array([31u8; 32]),
                tick_array_lower: Pubkey::new_from_array([32u8; 32]),
                tick_array_upper: Pubkey::new_from_array([33u8; 32]),
            },
        );
        assert_eq!(ix.program_id, program_id);
        assert_eq!(ix.data, disc::UPDATE_FEES_AND_REWARDS.to_vec());
        assert_eq!(ix.accounts.len(), 4);
        let flags: Vec<(bool, bool)> = ix
            .accounts
            .iter()
            .map(|a| (a.is_signer, a.is_writable))
            .collect();
        assert_eq!(
            flags,
            vec![
                (false, true),  // whirlpool (writable — accumulators bump)
                (false, true),  // position (writable — checkpoints bump)
                (false, false), // tick_array_lower
                (false, false), // tick_array_upper
            ]
        );
    }

    #[tokio::test]
    async fn submit_rebalance_bails_until_31b() {
        // Verify the stub returns a clear error rather than panicking.
        // This is the contract `RebalanceEngine::execute` relies on:
        // any `Err` here propagates up and leaves the journal row in
        // Pending — exactly the recovery-friendly state #32 wants.
        use crate::execution::rebalance::TickRange;
        use crate::strategy::range::RangeRecommendation;
        use crate::strategy::signal::RebalanceReason;

        let submitter = WhirlpoolTxSubmitter::mainnet().unwrap();
        let plan = RebalancePlan {
            position_id: 1,
            position_mint: Pubkey::new_unique(),
            current_range: TickRange::new(-100, 100).unwrap(),
            target_range: RangeRecommendation {
                lower_tick: -200,
                upper_tick: 200,
                expected_capital_efficiency_ppm: 5_000_000,
            },
            reason: RebalanceReason::OutOfRange,
            started_at_secs: 0,
        };
        let err = submitter.submit_rebalance(&plan).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("#31b"), "{msg}");
        assert!(msg.contains("not yet implemented"), "{msg}");
    }
}
