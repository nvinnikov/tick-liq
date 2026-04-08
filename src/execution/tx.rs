//! Transaction signer and submitter.
//!
//! Loads a Solana keypair from environment-only sources (per CLAUDE.md:
//! "Keypairs only via environment variables, never in config files or
//! code"), builds a `Transaction` from a caller-supplied instruction
//! list, attaches a compute-budget priority-fee instruction, signs it,
//! submits to RPC, and waits for confirmation. On `BlockhashNotFound`
//! at submit time we refresh the blockhash and retry up to a small
//! bounded number of attempts — this is the dominant non-deterministic
//! failure mode for happy-path submission and is safe to retry because
//! the transaction has not yet landed on-chain.
//!
//! ## Keypair sources
//!
//! Two env vars, in priority order:
//!   1. `KEYPAIR_PATH` — path to a Solana CLI JSON keypair file
//!      (a JSON array of 64 bytes: 32-byte seed + 32-byte pubkey, the
//!      same format `solana-keygen new` writes).
//!   2. `KEYPAIR_SEED` — a base58-encoded 64-byte secret key. Useful
//!      for ephemeral test keys and CI; production should always use
//!      `KEYPAIR_PATH` so the secret never lives in process env.
//!
//! Both are read by [`load_keypair_from_env`]. We do **not** read these
//! from `Config` directly because `Config::keypair_path` is optional —
//! the submitter explicitly requires one and we want the error to
//! surface here, at the layer that actually needs the secret.
//!
//! ## Why this is not the rebalance `TxSubmitter` impl
//!
//! [`TxSigner`] is the generic signer/submitter for any
//! `Vec<Instruction>`. The CLMM-specific instruction builder
//! (close → collect → open against the Whirlpool program) lives behind
//! the [`crate::execution::rebalance::TxSubmitter`] trait — see task
//! #31. That implementation will *use* `TxSigner` internally to sign
//! and ship the instructions it builds.

use anyhow::{anyhow, bail, Context, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::RpcSendTransactionConfig;
use solana_sdk::bs58;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::instruction::Instruction;
use solana_sdk::signature::{Keypair, Signature, Signer};
use solana_sdk::transaction::Transaction;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::data::rpc::RpcPool;

const ENV_KEYPAIR_PATH: &str = "KEYPAIR_PATH";
const ENV_KEYPAIR_SEED: &str = "KEYPAIR_SEED";

/// Default number of retry attempts on blockhash-expiry submit failures.
/// Two retries (three total attempts) covers the common case where the
/// fetched blockhash expires between fetch and submit on a slow path.
pub const DEFAULT_MAX_BLOCKHASH_RETRIES: u8 = 2;

/// Default compute-unit price in micro-lamports for priority fees.
/// `1_000` µlam/CU on a 200k CU tx ≈ 0.0002 SOL extra. Conservative
/// default that gets in front of the no-priority-fee crowd without
/// burning lamports on a healthy network. Tune up under congestion.
pub const DEFAULT_COMPUTE_UNIT_PRICE_MICROLAMPORTS: u64 = 1_000;

/// Default compute-unit limit. Solana's runtime default is 200k; we
/// pin it explicitly so the limit is part of the signed transaction
/// and not subject to client-side defaults drifting.
pub const DEFAULT_COMPUTE_UNIT_LIMIT: u32 = 200_000;

/// Configuration knobs for [`TxSigner`].
#[derive(Debug, Clone, Copy)]
pub struct TxSignerConfig {
    pub max_blockhash_retries: u8,
    pub compute_unit_price_microlamports: u64,
    pub compute_unit_limit: u32,
    pub commitment: CommitmentConfig,
}

impl Default for TxSignerConfig {
    fn default() -> Self {
        Self {
            max_blockhash_retries: DEFAULT_MAX_BLOCKHASH_RETRIES,
            compute_unit_price_microlamports: DEFAULT_COMPUTE_UNIT_PRICE_MICROLAMPORTS,
            compute_unit_limit: DEFAULT_COMPUTE_UNIT_LIMIT,
            commitment: CommitmentConfig::confirmed(),
        }
    }
}

/// Signs and submits transactions on behalf of the LP manager.
///
/// Cloneable: the inner `Keypair` is wrapped in `Arc` so multiple
/// async tasks can share a single signer without copying secret bytes.
#[derive(Clone)]
pub struct TxSigner {
    keypair: Arc<Keypair>,
    rpc: RpcPool,
    config: TxSignerConfig,
}

impl TxSigner {
    /// Build a signer with default configuration.
    pub fn new(keypair: Keypair, rpc: RpcPool) -> Self {
        Self::with_config(keypair, rpc, TxSignerConfig::default())
    }

    pub fn with_config(keypair: Keypair, rpc: RpcPool, config: TxSignerConfig) -> Self {
        Self {
            keypair: Arc::new(keypair),
            rpc,
            config,
        }
    }

    /// Pubkey of the signing keypair.
    pub fn pubkey(&self) -> solana_sdk::pubkey::Pubkey {
        self.keypair.pubkey()
    }

    /// Sign and submit a list of instructions, returning the transaction
    /// signature once the cluster confirms at the configured commitment.
    ///
    /// Retries up to `config.max_blockhash_retries` times on
    /// `BlockhashNotFound` errors at submit time. Other errors propagate
    /// immediately — they typically indicate a real problem (insufficient
    /// funds, program error, simulation failure) that retrying will not
    /// fix.
    pub async fn submit(&self, instructions: Vec<Instruction>) -> Result<Signature> {
        if instructions.is_empty() {
            bail!("submit called with empty instruction list");
        }
        let with_budget = self.with_compute_budget(instructions);

        let raw = self.rpc.raw();
        let mut last_err: Option<anyhow::Error> = None;
        let attempts = self.config.max_blockhash_retries.saturating_add(1);
        for attempt in 0..attempts {
            match self.try_submit_once(raw, &with_budget).await {
                Ok(sig) => return Ok(sig),
                Err(e) => {
                    if attempt + 1 < attempts && is_blockhash_expired(&e) {
                        tracing::warn!(
                            attempt = attempt + 1,
                            "blockhash expired, refreshing and retrying"
                        );
                        // Brief backoff so we don't hammer the RPC if
                        // the cluster is genuinely behind.
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("submit exhausted retries")))
    }

    fn with_compute_budget(&self, mut instructions: Vec<Instruction>) -> Vec<Instruction> {
        // Prepend so callers' instruction order remains stable.
        let price = ComputeBudgetInstruction::set_compute_unit_price(
            self.config.compute_unit_price_microlamports,
        );
        let limit =
            ComputeBudgetInstruction::set_compute_unit_limit(self.config.compute_unit_limit);
        let mut prefixed = Vec::with_capacity(instructions.len() + 2);
        prefixed.push(limit);
        prefixed.push(price);
        prefixed.append(&mut instructions);
        prefixed
    }

    async fn try_submit_once(
        &self,
        raw: &RpcClient,
        instructions: &[Instruction],
    ) -> Result<Signature> {
        let _permit = self.rpc.acquire().await?;
        let blockhash = raw
            .get_latest_blockhash()
            .await
            .context("get_latest_blockhash")?;
        let payer = self.keypair.pubkey();
        let mut tx = Transaction::new_with_payer(instructions, Some(&payer));
        tx.try_sign(&[self.keypair.as_ref()], blockhash)
            .context("sign transaction")?;
        let sig = raw
            .send_transaction_with_config(
                &tx,
                RpcSendTransactionConfig {
                    skip_preflight: false,
                    preflight_commitment: Some(self.config.commitment.commitment),
                    ..Default::default()
                },
            )
            .await
            .context("send_transaction")?;
        raw.confirm_transaction_with_commitment(&sig, self.config.commitment)
            .await
            .context("confirm_transaction")?;
        Ok(sig)
    }
}

/// Heuristic check: did this error come from the cluster reporting a
/// stale/unknown blockhash? `solana-client` surfaces this as a string
/// in the RPC error body — there's no typed variant we can match on at
/// this layer without depending on private error structs, so we look
/// for the well-known substrings the runtime uses.
fn is_blockhash_expired(err: &anyhow::Error) -> bool {
    let s = err.to_string();
    s.contains("BlockhashNotFound") || s.contains("blockhash not found")
}

/// Load a keypair from environment variables. Tries `KEYPAIR_PATH`
/// first (JSON file written by `solana-keygen`), then `KEYPAIR_SEED`
/// (base58-encoded 64-byte secret). Errors if neither is set.
pub fn load_keypair_from_env() -> Result<Keypair> {
    if let Ok(path) = std::env::var(ENV_KEYPAIR_PATH) {
        return load_keypair_from_file(Path::new(&path));
    }
    if let Ok(seed) = std::env::var(ENV_KEYPAIR_SEED) {
        return load_keypair_from_base58(&seed);
    }
    bail!("no keypair source set: expected {ENV_KEYPAIR_PATH} or {ENV_KEYPAIR_SEED} env var")
}

/// Load a keypair from a Solana CLI JSON file (a 64-element JSON byte
/// array). This is the format `solana-keygen new -o key.json` produces.
pub fn load_keypair_from_file(path: &Path) -> Result<Keypair> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read keypair file {}", path.display()))?;
    let arr: Vec<u8> = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse keypair file {} as JSON byte array", path.display()))?;
    if arr.len() != 64 {
        bail!(
            "keypair file {} must contain 64 bytes, got {}",
            path.display(),
            arr.len()
        );
    }
    Keypair::from_bytes(&arr).context("construct Keypair from file bytes")
}

/// Load a keypair from a base58-encoded 64-byte secret string.
pub fn load_keypair_from_base58(s: &str) -> Result<Keypair> {
    let bytes = bs58::decode(s.trim())
        .into_vec()
        .context("base58-decode keypair seed")?;
    if bytes.len() != 64 {
        bail!(
            "base58 keypair must decode to 64 bytes, got {}",
            bytes.len()
        );
    }
    Keypair::from_bytes(&bytes).context("construct Keypair from base58 bytes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signer::Signer;

    fn write_keypair_file(kp: &Keypair) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "tick-liq-tx-test-{}-{}.json",
            std::process::id(),
            kp.pubkey()
        ));
        let bytes: Vec<u8> = kp.to_bytes().to_vec();
        let json = serde_json::to_vec(&bytes).unwrap();
        std::fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn load_keypair_from_file_round_trips() {
        let kp = Keypair::new();
        let expected = kp.pubkey();
        let path = write_keypair_file(&kp);
        let loaded = load_keypair_from_file(&path).unwrap();
        assert_eq!(loaded.pubkey(), expected);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_keypair_from_file_rejects_wrong_length() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("tick-liq-tx-bad-{}.json", std::process::id()));
        // 32 bytes instead of 64.
        let bytes: Vec<u8> = (0u8..32).collect();
        std::fs::write(&path, serde_json::to_vec(&bytes).unwrap()).unwrap();
        let err = load_keypair_from_file(&path).unwrap_err();
        assert!(
            err.to_string().contains("64 bytes"),
            "unexpected error: {err}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_keypair_from_file_rejects_missing() {
        let err = load_keypair_from_file(Path::new("/nonexistent/tick-liq-xyz.json")).unwrap_err();
        assert!(err.to_string().contains("read keypair file"), "{err}");
    }

    #[test]
    fn load_keypair_from_base58_round_trips() {
        let kp = Keypair::new();
        let expected = kp.pubkey();
        let s = bs58::encode(kp.to_bytes()).into_string();
        let loaded = load_keypair_from_base58(&s).unwrap();
        assert_eq!(loaded.pubkey(), expected);
    }

    #[test]
    fn load_keypair_from_base58_rejects_wrong_length() {
        let s = bs58::encode([1u8; 32]).into_string();
        let err = load_keypair_from_base58(&s).unwrap_err();
        assert!(err.to_string().contains("64 bytes"), "{err}");
    }

    #[test]
    fn load_keypair_from_base58_rejects_garbage() {
        let err = load_keypair_from_base58("not!base58!").unwrap_err();
        assert!(err.to_string().contains("base58-decode"), "{err}");
    }

    #[test]
    fn submit_empty_instructions_errors() {
        let kp = Keypair::new();
        let pool = RpcPool::new("http://127.0.0.1:1");
        let signer = TxSigner::new(kp, pool);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(signer.submit(vec![])).unwrap_err();
        assert!(err.to_string().contains("empty instruction list"), "{err}");
    }

    #[test]
    fn with_compute_budget_prepends_two_instructions() {
        let kp = Keypair::new();
        let pool = RpcPool::new("http://127.0.0.1:1");
        let signer = TxSigner::new(kp, pool);
        let payer = signer.pubkey();
        // Use a no-op transfer-style ix as a stand-in user instruction.
        let user_ix = solana_sdk::system_instruction::transfer(&payer, &payer, 1);
        let out = signer.with_compute_budget(vec![user_ix.clone()]);
        assert_eq!(out.len(), 3);
        // The two prefix ixs target the compute-budget program.
        assert_eq!(out[0].program_id, solana_sdk::compute_budget::id());
        assert_eq!(out[1].program_id, solana_sdk::compute_budget::id());
        // User ix preserved at the tail.
        assert_eq!(out[2], user_ix);
    }

    #[test]
    fn is_blockhash_expired_detects_known_strings() {
        assert!(is_blockhash_expired(&anyhow!(
            "RPC error: BlockhashNotFound"
        )));
        assert!(is_blockhash_expired(&anyhow!(
            "transaction blockhash not found"
        )));
        assert!(!is_blockhash_expired(&anyhow!("InsufficientFundsForFee")));
    }

    #[test]
    fn config_defaults_are_sensible() {
        let c = TxSignerConfig::default();
        assert_eq!(c.max_blockhash_retries, DEFAULT_MAX_BLOCKHASH_RETRIES);
        assert_eq!(
            c.compute_unit_price_microlamports,
            DEFAULT_COMPUTE_UNIT_PRICE_MICROLAMPORTS
        );
        assert_eq!(c.compute_unit_limit, DEFAULT_COMPUTE_UNIT_LIMIT);
        assert_eq!(c.commitment, CommitmentConfig::confirmed());
    }
}
