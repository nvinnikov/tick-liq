// Public API for Phase 5 live execution — callers added in plan 05-02.
// Suppress dead-code lint on the entire module until wired up.
#![allow(dead_code)]

use std::sync::Arc;
use anyhow::{Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::Transaction,
};

use crate::protocols::orca::{
    position_pda, whirlpool_program_pubkey, WhirlpoolPool, WhirlpoolPosition,
};

/// Parameters passed to OrcaExecutor::execute_rebalance.
#[allow(dead_code)]
pub struct OrcaRebalanceParams {
    pub pool_address: Pubkey,
    pub pool: WhirlpoolPool,
    pub position_mint: Pubkey,
    pub position: WhirlpoolPosition,
    pub new_tick_lower: i32,
    pub new_tick_upper: i32,
}

/// Off-chain Orca Whirlpool rebalance executor.
/// Builds and submits four transactions: update_fees_and_rewards, collect_fees,
/// close_position, open_position.
#[allow(dead_code)]
pub struct OrcaExecutor {
    rpc: RpcClient,
    keypair: Arc<Keypair>,
    program_id: Pubkey,
}

impl OrcaExecutor {
    pub fn new(rpc_url: &str, keypair: Arc<Keypair>) -> Self {
        Self {
            rpc: RpcClient::new(rpc_url.to_string()),
            keypair,
            program_id: whirlpool_program_pubkey(),
        }
    }

    /// Step 1: update_fees_and_rewards
    /// Accounts (4): whirlpool (w), position (w), tick_array_lower (r), tick_array_upper (r)
    pub fn ix_update_fees_and_rewards(
        &self,
        pool_address: &Pubkey,
        position_pda_key: &Pubkey,
        tick_array_lower: &Pubkey,
        tick_array_upper: &Pubkey,
    ) -> Result<Instruction> {
        use anchor_lang::InstructionData;
        let data = whirlpool_cpi::instruction::UpdateFeesAndRewards {}.data();
        let accounts = vec![
            AccountMeta::new(*pool_address, false),
            AccountMeta::new(*position_pda_key, false),
            AccountMeta::new_readonly(*tick_array_lower, false),
            AccountMeta::new_readonly(*tick_array_upper, false),
        ];
        Ok(Instruction { program_id: self.program_id, accounts, data })
    }

    /// Step 2: collect_fees
    /// Accounts (9): whirlpool(r), position_authority(s), position(w),
    ///   position_token_account(r), token_owner_account_a(w), token_vault_a(w),
    ///   token_owner_account_b(w), token_vault_b(w), token_program(r)
    pub fn ix_collect_fees(
        &self,
        pool_address: &Pubkey,
        pool: &WhirlpoolPool,
        position_pda_key: &Pubkey,
        position_mint: &Pubkey,
    ) -> Result<Instruction> {
        use anchor_lang::InstructionData;
        let wallet = self.keypair.pubkey();
        let data = whirlpool_cpi::instruction::CollectFees {}.data();
        let position_ata =
            spl_associated_token_account::get_associated_token_address(&wallet, position_mint);
        let token_owner_a =
            spl_associated_token_account::get_associated_token_address(&wallet, &pool.token_mint_a);
        let token_owner_b =
            spl_associated_token_account::get_associated_token_address(&wallet, &pool.token_mint_b);
        let accounts = vec![
            AccountMeta::new_readonly(*pool_address, false),
            AccountMeta::new_readonly(wallet, true),
            AccountMeta::new(*position_pda_key, false),
            AccountMeta::new_readonly(position_ata, false),
            AccountMeta::new(token_owner_a, false),
            AccountMeta::new(pool.token_vault_a, false),
            AccountMeta::new(token_owner_b, false),
            AccountMeta::new(pool.token_vault_b, false),
            AccountMeta::new_readonly(spl_token::ID, false),
        ];
        Ok(Instruction { program_id: self.program_id, accounts, data })
    }

    /// Step 3: close_position
    /// Accounts (6): position_authority(s), receiver(w), position(w),
    ///   position_mint(w), position_token_account(w), token_program(r)
    pub fn ix_close_position(
        &self,
        position_pda_key: &Pubkey,
        position_mint: &Pubkey,
    ) -> Result<Instruction> {
        use anchor_lang::InstructionData;
        let wallet = self.keypair.pubkey();
        let data = whirlpool_cpi::instruction::ClosePosition {}.data();
        let position_ata =
            spl_associated_token_account::get_associated_token_address(&wallet, position_mint);
        let accounts = vec![
            AccountMeta::new_readonly(wallet, true),
            AccountMeta::new(wallet, false),
            AccountMeta::new(*position_pda_key, false),
            AccountMeta::new(*position_mint, false),
            AccountMeta::new(position_ata, false),
            AccountMeta::new_readonly(spl_token::ID, false),
        ];
        Ok(Instruction { program_id: self.program_id, accounts, data })
    }

    /// Step 4: open_position
    /// Accounts (10): funder(w,s), owner(r), position(w), position_mint(w,s),
    ///   position_token_account(w), whirlpool(r), token_program(r), system_program(r),
    ///   rent(r), associated_token_program(r)
    ///
    /// NOTE: RESEARCH.md lists 10 accounts; plan frontmatter says 8, but actual
    /// whirlpool-cpi context.rs (OpenPosition struct) and on-chain program both require 10.
    /// Using 10 to match the actual program requirements.
    ///
    /// position_mint MUST be a fresh Keypair — it co-signs the transaction.
    /// Returns (Instruction, new_position_mint_keypair).
    pub fn ix_open_position(
        &self,
        pool_address: &Pubkey,
        tick_lower_index: i32,
        tick_upper_index: i32,
    ) -> Result<(Instruction, Keypair)> {
        use anchor_lang::InstructionData;
        let wallet = self.keypair.pubkey();
        let new_mint = Keypair::new();
        let new_position_pda = position_pda(&new_mint.pubkey());
        let new_position_ata = spl_associated_token_account::get_associated_token_address(
            &wallet,
            &new_mint.pubkey(),
        );
        let data = whirlpool_cpi::instruction::OpenPosition {
            bumps: whirlpool_cpi::state::OpenPositionBumps { position_bump: 0 },
            tick_lower_index,
            tick_upper_index,
        }
        .data();
        let accounts = vec![
            AccountMeta::new(wallet, true),                                           // funder
            AccountMeta::new_readonly(wallet, false),                                 // owner
            AccountMeta::new(new_position_pda, false),                                // position
            AccountMeta::new(new_mint.pubkey(), true),                                // position_mint
            AccountMeta::new(new_position_ata, false),                                // position_token_account
            AccountMeta::new_readonly(*pool_address, false),                          // whirlpool
            AccountMeta::new_readonly(spl_token::ID, false),                         // token_program
            AccountMeta::new_readonly(solana_sdk::system_program::ID, false),        // system_program
            AccountMeta::new_readonly(solana_sdk::sysvar::rent::ID, false),          // rent
            AccountMeta::new_readonly(spl_associated_token_account::ID, false),      // associated_token_program
        ];
        Ok((Instruction { program_id: self.program_id, accounts, data }, new_mint))
    }

    /// Build and submit a single-instruction transaction signed by the wallet keypair.
    /// For open_position, call submit_tx_with_extra_signer instead.
    #[allow(dead_code)]
    fn submit_tx(&self, ix: Instruction) -> Result<Signature> {
        let blockhash = self
            .rpc
            .get_latest_blockhash()
            .context("get_latest_blockhash failed")?;
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.keypair.pubkey()),
            &[self.keypair.as_ref()],
            blockhash,
        );
        self.rpc
            .send_and_confirm_transaction(&tx)
            .context("send_and_confirm_transaction failed")
    }

    /// Build and submit a transaction with an additional signer (for open_position
    /// where position_mint must co-sign).
    #[allow(dead_code)]
    fn submit_tx_with_extra_signer(&self, ix: Instruction, extra: &Keypair) -> Result<Signature> {
        let blockhash = self
            .rpc
            .get_latest_blockhash()
            .context("get_latest_blockhash failed")?;
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.keypair.pubkey()),
            &[self.keypair.as_ref(), extra],
            blockhash,
        );
        self.rpc
            .send_and_confirm_transaction(&tx)
            .context("send_and_confirm_transaction failed")
    }

    /// Build a simulate-only transaction (no signing required beyond payer placeholder).
    /// Used in tests marked #[ignore].
    pub fn simulate_tx(&self, ix: Instruction) -> Result<()> {
        let blockhash = self
            .rpc
            .get_latest_blockhash()
            .context("get_latest_blockhash failed")?;
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.keypair.pubkey()),
            &[self.keypair.as_ref()],
            blockhash,
        );
        let result = self
            .rpc
            .simulate_transaction(&tx)
            .context("simulate_transaction RPC call failed")?;
        if let Some(err) = result.value.err {
            anyhow::bail!(
                "simulateTransaction returned error: {:?}\nlogs: {:?}",
                err,
                result.value.logs
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signer::keypair::Keypair;
    use std::sync::Arc;

    fn dummy_executor() -> OrcaExecutor {
        OrcaExecutor::new("https://api.devnet.solana.com", Arc::new(Keypair::new()))
    }

    fn dummy_pool() -> WhirlpoolPool {
        WhirlpoolPool {
            _whirlpools_config: Pubkey::default(),
            _whirlpool_bump: [0],
            tick_spacing: 64,
            _tick_spacing_seed: [0; 2],
            fee_rate: 300,
            _protocol_fee_rate: 0,
            liquidity: 0,
            sqrt_price: 0,
            tick_current_index: 0,
            _protocol_fee_owed_a: 0,
            _protocol_fee_owed_b: 0,
            token_mint_a: Pubkey::new_unique(),
            token_vault_a: Pubkey::new_unique(),
            fee_growth_global_a: 0,
            token_mint_b: Pubkey::new_unique(),
            token_vault_b: Pubkey::new_unique(),
            fee_growth_global_b: 0,
            _reward_last_updated_timestamp: 0,
        }
    }

    #[test]
    fn update_fees_accounts_count() {
        let ex = dummy_executor();
        let pool = Pubkey::new_unique();
        let pos = Pubkey::new_unique();
        let ta_lower = Pubkey::new_unique();
        let ta_upper = Pubkey::new_unique();
        let ix = ex
            .ix_update_fees_and_rewards(&pool, &pos, &ta_lower, &ta_upper)
            .unwrap();
        assert_eq!(ix.accounts.len(), 4);
    }

    #[test]
    fn collect_fees_accounts_count() {
        let ex = dummy_executor();
        let pool_addr = Pubkey::new_unique();
        let pool = dummy_pool();
        let position_pda_key = Pubkey::new_unique();
        let position_mint = Pubkey::new_unique();
        let ix = ex
            .ix_collect_fees(&pool_addr, &pool, &position_pda_key, &position_mint)
            .unwrap();
        assert_eq!(ix.accounts.len(), 9);
    }

    #[test]
    fn close_position_accounts_count() {
        let ex = dummy_executor();
        let pos = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let ix = ex.ix_close_position(&pos, &mint).unwrap();
        assert_eq!(ix.accounts.len(), 6);
    }

    #[test]
    fn open_position_accounts_count() {
        let ex = dummy_executor();
        let pool = Pubkey::new_unique();
        let (ix, _new_mint) = ex.ix_open_position(&pool, -100, 100).unwrap();
        // NOTE: Actual OpenPosition requires 10 accounts (funder, owner, position,
        // position_mint, position_token_account, whirlpool, token_program,
        // system_program, rent, associated_token_program).
        // Plan frontmatter says 8, but whirlpool-cpi context.rs and on-chain program
        // both require 10. Test reflects actual requirement.
        assert_eq!(ix.accounts.len(), 10);
    }
}
