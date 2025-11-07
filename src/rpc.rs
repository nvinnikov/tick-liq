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

    /// Fetch raw account bytes. Returns error if account not found.
    pub fn fetch_account_data(&self, address: &str) -> Result<Vec<u8>> {
        let pubkey = Pubkey::from_str(address)
            .map_err(|e| anyhow!("Invalid address '{}': {}", address, e))?;

        let account = self.client
            .get_account(&pubkey)
            .map_err(|e| anyhow!("Account '{}' not found: {}", address, e))?;

        Ok(account.data)
    }

    /// Fetch account program owner. Used to verify account type before deserializing.
    pub fn fetch_owner(&self, address: &str) -> Result<Pubkey> {
        let pubkey = Pubkey::from_str(address)
            .map_err(|e| anyhow!("Invalid address '{}': {}", address, e))?;

        let account = self.client
            .get_account(&pubkey)
            .map_err(|e| anyhow!("Account '{}' not found: {}", address, e))?;

        Ok(account.owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_address_returns_error() {
        let rpc = SolanaRpc::new("https://api.devnet.solana.com");
        let result = rpc.fetch_account_data("not_a_valid_pubkey");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid address"));
    }
}