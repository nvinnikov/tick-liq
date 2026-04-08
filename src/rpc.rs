use anyhow::{anyhow, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

pub struct SolanaRpc {
    pub client: RpcClient,
}

impl SolanaRpc {
    pub fn new(url: &str) -> Self {
        Self {
            client: RpcClient::new(url.to_string()),
        }
    }

    /// Fetch account bytes and verify the program owner matches `expected_owner`.
    ///
    /// CLAUDE.md mandate: "Always verify program owner before deserializing."
    /// This protects borsh deserialization from being run on attacker-controlled
    /// or unrelated accounts that happen to deserialize without erroring.
    pub fn fetch_account_checked(&self, address: &str, expected_owner: &Pubkey) -> Result<Vec<u8>> {
        let pubkey = Pubkey::from_str(address)
            .map_err(|e| anyhow!("Invalid address '{}': {}", address, e))?;

        let account = self
            .client
            .get_account(&pubkey)
            .map_err(|e| anyhow!("Account '{}' not found: {}", address, e))?;

        verify_owner(address, &account.owner, expected_owner)?;
        Ok(account.data)
    }
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
}
