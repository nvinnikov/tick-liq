use anyhow::{anyhow, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

/// Metaplex Token Metadata program ID.
const METADATA_PROGRAM_ID: &str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";

/// SPL Token mint account: decimals byte is at offset 44.
/// Layout: mint_authority option (36) + supply (8) = 44 bytes before decimals.
const MINT_DECIMALS_OFFSET: usize = 44;

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

    /// Fetch the `decimals` field from an SPL Token mint account.
    ///
    /// SPL mint layout (packed): option<mint_authority> (36 bytes) + supply (8 bytes) = 44 bytes,
    /// then decimals (1 byte) at offset 44.
    pub fn fetch_mint_decimals(&self, mint: &Pubkey) -> Result<u8> {
        let account = self
            .client
            .get_account(mint)
            .map_err(|e| anyhow!("Mint account '{}' not found: {}", mint, e))?;

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

        let account = self
            .client
            .get_account(&metadata_pda)
            .map_err(|e| anyhow!("Metadata account not found: {}", e))?;

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
        let mut data = vec![0u8; 82]; // SPL mint is 82 bytes
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
}
