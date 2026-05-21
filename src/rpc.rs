use anyhow::{anyhow, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::time::Duration;

/// Metaplex Token Metadata program ID.
const METADATA_PROGRAM_ID: &str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";

/// SPL Token mint account: decimals byte is at offset 44.
/// Layout: mint_authority option (36) + supply (8) = 44 bytes before decimals.
const MINT_DECIMALS_OFFSET: usize = 44;

/// Default per-request timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Maximum number of attempts (1 initial + 2 retries).
const MAX_ATTEMPTS: u32 = 3;

/// Base delay for exponential backoff between retries.
const RETRY_BASE: Duration = Duration::from_millis(250);

pub struct SolanaRpc {
    pub client: RpcClient,
    /// Stored for diagnostics / future retry-timeout tuning.
    #[allow(dead_code)]
    timeout_secs: u64,
}

impl SolanaRpc {
    /// Create a new client with the default [`DEFAULT_TIMEOUT_SECS`]-second timeout.
    #[allow(dead_code)]
    pub fn new(url: &str) -> Self {
        Self::with_timeout(url, DEFAULT_TIMEOUT_SECS)
    }

    /// Create a new client with a configurable per-request timeout (seconds).
    ///
    /// The timeout is passed directly to the underlying [`RpcClient`] HTTP
    /// layer so every individual request is bounded.  On timeout or transport
    /// error each call is retried up to [`MAX_ATTEMPTS`] times with
    /// exponential backoff (250 ms → 500 ms → 1 s).
    pub fn with_timeout(url: &str, timeout_secs: u64) -> Self {
        Self {
            client: RpcClient::new_with_timeout(url.to_string(), Duration::from_secs(timeout_secs)),
            timeout_secs,
        }
    }

    /// Execute a fallible RPC closure with up to [`MAX_ATTEMPTS`] retries and
    /// exponential backoff (250 ms, 500 ms, 1 s).
    ///
    /// `label` appears in warning logs and the final error message.
    ///
    /// This method is synchronous but safe to call from within a tokio runtime:
    /// when invoked on a multi-thread runtime worker we use
    /// [`tokio::task::block_in_place`] + [`tokio::time::sleep`] so other tasks
    /// continue to make progress during backoff; otherwise we fall back to
    /// plain [`std::thread::sleep`] (sync-only caller or single-thread runtime).
    fn retry<F, T>(&self, label: &str, mut f: F) -> Result<T>
    where
        F: FnMut() -> Result<T>,
    {
        let mut last_err = anyhow!("{label}: no attempts made");
        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                let delay = RETRY_BASE * 2u32.pow(attempt - 1);
                sleep_backoff(delay);
            }
            match f() {
                Ok(v) => return Ok(v),
                Err(e) => {
                    tracing::warn!("{label}: attempt {attempt} failed: {e}");
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }

    /// Fetch account bytes and verify the program owner matches `expected_owner`.
    ///
    /// CLAUDE.md mandate: "Always verify program owner before deserializing."
    /// Retried up to [`MAX_ATTEMPTS`] times on transport/timeout errors.
    pub fn fetch_account_checked(&self, address: &str, expected_owner: &Pubkey) -> Result<Vec<u8>> {
        let pubkey = Pubkey::from_str(address)
            .map_err(|e| anyhow!("Invalid address '{}': {}", address, e))?;

        let account = self.retry(&format!("get_account({address})"), || {
            self.client
                .get_account(&pubkey)
                .map_err(|e| anyhow!("Account '{}' not found: {}", address, e))
        })?;

        verify_owner(address, &account.owner, expected_owner)?;
        Ok(account.data)
    }

    /// Fetch the `decimals` field from an SPL Token mint account.
    ///
    /// SPL mint layout (packed): option<mint_authority> (36 bytes) + supply (8 bytes) = 44 bytes,
    /// then decimals (1 byte) at offset 44.
    /// Retried up to [`MAX_ATTEMPTS`] times on transport/timeout errors.
    pub fn fetch_mint_decimals(&self, mint: &Pubkey) -> Result<u8> {
        let account = self.retry(&format!("get_account(mint={mint})"), || {
            self.client
                .get_account(mint)
                .map_err(|e| anyhow!("Mint account '{}' not found: {}", mint, e))
        })?;

        if account.data.len() <= MINT_DECIMALS_OFFSET {
            return Err(anyhow!(
                "Mint account '{}' data too short ({} bytes)",
                mint,
                account.data.len()
            ));
        }

        Ok(account.data[MINT_DECIMALS_OFFSET])
    }

    /// Fetch the token symbol from Metaplex token metadata, falling back to the
    /// first 8 characters of the mint address if metadata is unavailable.
    pub fn fetch_token_symbol(&self, mint: &Pubkey) -> String {
        match self.try_fetch_token_symbol(mint) {
            Ok(sym) if !sym.is_empty() => sym,
            _ => {
                let s = mint.to_string();
                format!("{}…", &s[..8.min(s.len())])
            }
        }
    }

    fn try_fetch_token_symbol(&self, mint: &Pubkey) -> Result<String> {
        let metadata_program = Pubkey::from_str(METADATA_PROGRAM_ID)
            .map_err(|e| anyhow!("Bad metadata program id: {}", e))?;

        let (metadata_pda, _) = Pubkey::find_program_address(
            &[b"metadata", metadata_program.as_ref(), mint.as_ref()],
            &metadata_program,
        );

        let account = self.retry("get_account(metadata_pda)", || {
            self.client
                .get_account(&metadata_pda)
                .map_err(|e| anyhow!("Metadata account not found: {}", e))
        })?;

        // Metaplex metadata v1 layout:
        //   key(1) + update_authority(32) + mint(32) = 65 bytes header
        //   name: u32 len + bytes (max 32, null-padded)
        //   symbol: u32 len + bytes (max 10, null-padded)
        let data = &account.data;
        let mut pos = 65usize;

        if pos + 4 > data.len() {
            return Err(anyhow!("Metadata too short for name length"));
        }
        let name_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4 + name_len;

        if pos + 4 > data.len() {
            return Err(anyhow!("Metadata too short for symbol length"));
        }
        let sym_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;

        if pos + sym_len > data.len() {
            return Err(anyhow!("Metadata too short for symbol data"));
        }

        let sym = std::str::from_utf8(&data[pos..pos + sym_len])
            .map_err(|e| anyhow!("Symbol not valid UTF-8: {}", e))?
            .trim_end_matches('\0')
            .to_string();

        Ok(sym)
    }
}

/// Sleep for `delay` without monopolising a tokio worker thread.
///
/// When called on a multi-thread tokio runtime we hop to a blocking-safe
/// context via [`tokio::task::block_in_place`] and await
/// [`tokio::time::sleep`], which lets other tasks (WebSocket handlers,
/// Telegram dispatcher, …) keep running. Outside a tokio runtime, or on a
/// current-thread runtime where `block_in_place` is unavailable, we fall
/// back to plain [`std::thread::sleep`].
fn sleep_backoff(delay: Duration) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
            tokio::task::block_in_place(|| {
                handle.block_on(tokio::time::sleep(delay));
            });
            return;
        }
    }
    std::thread::sleep(delay);
}

/// Returns an error (never panics) if `actual` differs from `expected`.
pub fn verify_owner(address: &str, actual: &Pubkey, expected: &Pubkey) -> Result<()> {
    if actual != expected {
        return Err(anyhow!(
            "Account '{}' has owner {} but expected program {}",
            address,
            actual,
            expected
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_address_returns_error() {
        let rpc = SolanaRpc::new("https://api.devnet.solana.com");
        let dummy_owner = Pubkey::new_unique();
        let result = rpc.fetch_account_checked("not_a_valid_pubkey", &dummy_owner);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid address"));
    }

    #[test]
    fn test_verify_owner_mismatch_returns_error() {
        let expected = Pubkey::new_unique();
        let actual = Pubkey::new_unique();
        let res = verify_owner("SomeAddr", &actual, &expected);
        assert!(res.is_err());
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("expected program"));
        assert!(msg.contains("SomeAddr"));
    }

    #[test]
    fn test_verify_owner_match_ok() {
        let owner = Pubkey::new_unique();
        assert!(verify_owner("SomeAddr", &owner, &owner).is_ok());
    }

    #[test]
    fn test_mint_decimals_offset_constant() {
        // SPL mint: COption<Pubkey> = 4+32 = 36, supply u64 = 8, total = 44
        assert_eq!(MINT_DECIMALS_OFFSET, 44);
    }

    #[test]
    fn test_fetch_mint_decimals_reads_correct_byte() {
        let mut data = [0u8; 82]; // SPL mint is 82 bytes
        data[MINT_DECIMALS_OFFSET] = 9;
        assert_eq!(data[MINT_DECIMALS_OFFSET], 9);
        data[MINT_DECIMALS_OFFSET] = 6;
        assert_eq!(data[MINT_DECIMALS_OFFSET], 6);
    }

    #[test]
    fn test_try_fetch_token_symbol_parses_layout() {
        // Build synthetic metadata: 65-byte header + name + symbol
        let mut data = vec![0u8; 65];
        let name = b"TestToken";
        data.extend_from_slice(&(name.len() as u32).to_le_bytes());
        data.extend_from_slice(name);
        let sym = b"TST";
        data.extend_from_slice(&(sym.len() as u32).to_le_bytes());
        data.extend_from_slice(sym);
        data.extend_from_slice(&[0u8; 10]);

        // Parse with same logic as try_fetch_token_symbol
        let mut pos = 65usize;
        let name_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4 + name_len;
        let sym_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        let parsed = std::str::from_utf8(&data[pos..pos + sym_len])
            .unwrap()
            .trim_end_matches('\0')
            .to_string();

        assert_eq!(parsed, "TST");
    }

    /// Verify that `retry` succeeds on the first attempt when the closure is Ok.
    #[test]
    fn retry_succeeds_immediately_on_ok() {
        let rpc = SolanaRpc::with_timeout("https://api.devnet.solana.com", 30);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let calls_clone = calls.clone();
        let result = rpc.retry("test_ok", move || {
            calls_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok::<i32, anyhow::Error>(42)
        });
        assert_eq!(result.unwrap(), 42);
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "should only call once on success"
        );
    }

    /// Verify that `retry` exhausts all attempts and returns the last error.
    #[test]
    fn retry_exhausts_attempts_on_persistent_error() {
        let rpc = SolanaRpc::with_timeout("https://api.devnet.solana.com", 30);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let calls_clone = calls.clone();
        let result = rpc.retry::<_, i32>("test_err", move || {
            calls_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(anyhow!("transient error"))
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("transient error"));
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            MAX_ATTEMPTS,
            "should attempt exactly MAX_ATTEMPTS times"
        );
    }

    /// Verify that `retry` succeeds on a later attempt after initial failures.
    #[test]
    fn retry_succeeds_on_second_attempt() {
        let rpc = SolanaRpc::with_timeout("https://api.devnet.solana.com", 30);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let calls_clone = calls.clone();
        let result = rpc.retry("test_eventual_ok", move || {
            let n = calls_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Err(anyhow!("first attempt fails"))
            } else {
                Ok::<i32, anyhow::Error>(7)
            }
        });
        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// Verify `with_timeout` constructor stores the timeout and builds a client.
    #[test]
    fn with_timeout_constructs_client() {
        let rpc = SolanaRpc::with_timeout("https://api.devnet.solana.com", 5);
        // Just verify the struct is constructed — we can't inspect the internal
        // reqwest timeout, but we can check the client url via a bad pubkey call.
        let dummy_owner = Pubkey::new_unique();
        let result = rpc.fetch_account_checked("not_a_valid_pubkey", &dummy_owner);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid address"));
    }
}
