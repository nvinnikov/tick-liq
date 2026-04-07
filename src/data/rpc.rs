//! Pooled async Solana RPC client.
//!
//! Wraps `solana_client::nonblocking::rpc_client::RpcClient` in an `Arc` and
//! gates concurrent in-flight requests with a `Semaphore` so callers can fan
//! out work without overwhelming the upstream RPC endpoint.
//!
//! Typed helpers always return the account `owner` alongside the data so
//! callers can verify program ownership before deserializing — this is the
//! key safety rule from `CLAUDE.md` ("Always verify program owner before
//! deserializing").
//!
//! The legacy blocking client at `src/rpc.rs` is still used by the inspector
//! binary; this module is the forward-looking async client that the rest of
//! the LP manager will build on. Migration of the inspector is a separate
//! task.

use anyhow::{Context, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Default cap on concurrent in-flight RPC requests per pool. Mainnet public
/// endpoints typically rate-limit aggressively; private/dedicated nodes can
/// raise this via [`RpcPoolConfig`].
pub const DEFAULT_MAX_CONCURRENT: usize = 16;

/// Configuration for [`RpcPool::with_config`].
#[derive(Debug, Clone)]
pub struct RpcPoolConfig {
    pub url: String,
    pub commitment: CommitmentConfig,
    /// Maximum number of in-flight requests allowed at once.
    pub max_concurrent: usize,
}

impl RpcPoolConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            commitment: CommitmentConfig::confirmed(),
            max_concurrent: DEFAULT_MAX_CONCURRENT,
        }
    }
}

/// Owned account snapshot returned by [`RpcPool::fetch_account_data`].
///
/// Always carries the program `owner` so the caller can verify it before
/// trusting `data`.
#[derive(Debug, Clone)]
pub struct AccountSnapshot {
    pub data: Vec<u8>,
    pub owner: Pubkey,
    pub lamports: u64,
}

/// Pooled, cloneable async Solana RPC client.
///
/// Cloning is cheap — the inner [`RpcClient`] and semaphore are reference
/// counted, so multiple tasks can share a single pool.
#[derive(Clone)]
pub struct RpcPool {
    inner: Arc<RpcClient>,
    permits: Arc<Semaphore>,
}

impl RpcPool {
    /// Build a pool with default configuration (`confirmed` commitment,
    /// [`DEFAULT_MAX_CONCURRENT`] concurrent requests).
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_config(RpcPoolConfig::new(url))
    }

    /// Build a pool from explicit configuration.
    pub fn with_config(config: RpcPoolConfig) -> Self {
        let RpcPoolConfig {
            url,
            commitment,
            max_concurrent,
        } = config;
        let max_concurrent = max_concurrent.max(1);
        let inner = Arc::new(RpcClient::new_with_commitment(url, commitment));
        let permits = Arc::new(Semaphore::new(max_concurrent));
        Self { inner, permits }
    }

    /// Borrow the underlying nonblocking RPC client. Use this when the typed
    /// helpers in this module don't yet cover what you need; in that case
    /// acquire a permit yourself via [`RpcPool::acquire`] before issuing the
    /// call so concurrency limits are still honoured.
    pub fn raw(&self) -> &RpcClient {
        &self.inner
    }

    /// Acquire a single permit. Held until the returned guard is dropped.
    pub async fn acquire(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .context("rpc pool semaphore closed")
    }

    /// Fetch a single account, returning its data, owner, and lamports.
    ///
    /// Returns an error if the account does not exist or the RPC call fails.
    pub async fn fetch_account_data(&self, pubkey: &Pubkey) -> Result<AccountSnapshot> {
        let _permit = self.acquire().await?;
        let account = self
            .inner
            .get_account(pubkey)
            .await
            .with_context(|| format!("get_account({pubkey})"))?;
        Ok(AccountSnapshot {
            data: account.data,
            owner: account.owner,
            lamports: account.lamports,
        })
    }

    /// Fetch multiple accounts in a single RPC call.
    ///
    /// Returns one entry per requested pubkey, in the same order. Missing
    /// accounts are returned as `None`.
    pub async fn fetch_multiple_accounts(
        &self,
        pubkeys: &[Pubkey],
    ) -> Result<Vec<Option<AccountSnapshot>>> {
        if pubkeys.is_empty() {
            return Ok(Vec::new());
        }
        let _permit = self.acquire().await?;
        let accounts = self
            .inner
            .get_multiple_accounts(pubkeys)
            .await
            .with_context(|| format!("get_multiple_accounts({} keys)", pubkeys.len()))?;
        Ok(accounts
            .into_iter()
            .map(|maybe| {
                maybe.map(|a| AccountSnapshot {
                    data: a.data,
                    owner: a.owner,
                    lamports: a.lamports,
                })
            })
            .collect())
    }

    /// Fetch all accounts owned by `program_id`.
    ///
    /// This is an unbounded call on the upstream RPC and may be expensive or
    /// outright rejected by public endpoints. Prefer
    /// [`Self::fetch_multiple_accounts`] when you already know the keys.
    pub async fn fetch_program_accounts(
        &self,
        program_id: &Pubkey,
    ) -> Result<Vec<(Pubkey, AccountSnapshot)>> {
        let _permit = self.acquire().await?;
        let accounts = self
            .inner
            .get_program_accounts(program_id)
            .await
            .with_context(|| format!("get_program_accounts({program_id})"))?;
        Ok(accounts
            .into_iter()
            .map(|(pk, a)| {
                (
                    pk,
                    AccountSnapshot {
                        data: a.data,
                        owner: a.owner,
                        lamports: a.lamports,
                    },
                )
            })
            .collect())
    }

    /// Verify that `snapshot.owner == expected_owner`, returning the snapshot
    /// data on success. Use this immediately before deserializing any account
    /// payload (CLAUDE.md key note).
    pub fn verify_owner<'a>(
        snapshot: &'a AccountSnapshot,
        expected_owner: &Pubkey,
    ) -> Result<&'a [u8]> {
        anyhow::ensure!(
            snapshot.owner == *expected_owner,
            "account owner mismatch: expected {expected_owner}, got {}",
            snapshot.owner
        );
        Ok(&snapshot.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_pool() -> RpcPool {
        RpcPool::new("http://127.0.0.1:1") // never actually called
    }

    #[test]
    fn pool_is_clone_and_send_sync() {
        fn assert_send_sync<T: Send + Sync + Clone>() {}
        assert_send_sync::<RpcPool>();
        let pool = dummy_pool();
        let _clone = pool.clone();
    }

    #[test]
    fn config_defaults() {
        let cfg = RpcPoolConfig::new("https://api.devnet.solana.com");
        assert_eq!(cfg.url, "https://api.devnet.solana.com");
        assert_eq!(cfg.max_concurrent, DEFAULT_MAX_CONCURRENT);
        assert_eq!(cfg.commitment, CommitmentConfig::confirmed());
    }

    #[test]
    fn zero_max_concurrent_is_clamped_to_one() {
        let cfg = RpcPoolConfig {
            url: "http://x".into(),
            commitment: CommitmentConfig::processed(),
            max_concurrent: 0,
        };
        let pool = RpcPool::with_config(cfg);
        // Available permits should be at least 1 — pool is usable.
        assert!(pool.permits.available_permits() >= 1);
    }

    #[tokio::test]
    async fn semaphore_limits_concurrency() {
        let cfg = RpcPoolConfig {
            url: "http://x".into(),
            commitment: CommitmentConfig::processed(),
            max_concurrent: 2,
        };
        let pool = RpcPool::with_config(cfg);
        let _p1 = pool.acquire().await.unwrap();
        let _p2 = pool.acquire().await.unwrap();
        // Third acquire should not be immediately ready.
        let pool2 = pool.clone();
        let pending = tokio::spawn(async move {
            let _p3 = pool2.acquire().await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!pending.is_finished(), "third permit should be blocked");
        drop(_p1);
        // Now it should resolve.
        tokio::time::timeout(std::time::Duration::from_millis(200), pending)
            .await
            .expect("third permit should resolve after drop")
            .unwrap();
    }

    #[tokio::test]
    async fn fetch_multiple_accounts_empty_input_short_circuits() {
        let pool = dummy_pool();
        // Should return Ok(empty) WITHOUT attempting any network call.
        let result = pool.fetch_multiple_accounts(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn verify_owner_matches() {
        let owner = Pubkey::new_unique();
        let snap = AccountSnapshot {
            data: vec![1, 2, 3],
            owner,
            lamports: 0,
        };
        let data = RpcPool::verify_owner(&snap, &owner).unwrap();
        assert_eq!(data, &[1, 2, 3]);
    }

    #[test]
    fn verify_owner_rejects_mismatch() {
        let owner = Pubkey::new_unique();
        let other = Pubkey::new_unique();
        let snap = AccountSnapshot {
            data: vec![],
            owner,
            lamports: 0,
        };
        let err = RpcPool::verify_owner(&snap, &other).unwrap_err();
        assert!(err.to_string().contains("owner mismatch"), "{err}");
    }
}
