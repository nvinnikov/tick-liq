//! Local cache for per-position metadata that should persist across CLI sessions.
//!
//! Cache files are stored under the XDG data directory:
//!   `$XDG_DATA_HOME/lp-inspect/<mint>.json`
//! falling back to:
//!   `$HOME/.local/share/lp-inspect/<mint>.json`

use anyhow::{Result, anyhow};
use std::path::PathBuf;
use std::str::FromStr;

/// Return the path to the cache file for the given position mint.
///
/// The mint is validated as a base58 Solana pubkey before being used as a
/// file name, so a value like `../../etc/cron.d/x` can never escape the
/// cache directory regardless of where the mint string originated.
pub fn cache_path(mint: &str) -> Result<PathBuf> {
    let validated = solana_sdk::pubkey::Pubkey::from_str(mint)
        .map_err(|e| anyhow!("invalid mint '{}' for cache path: {}", mint, e))?;
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".local").join("share")
        });
    Ok(base.join("lp-inspect").join(format!("{}.json", validated)))
}

/// Load a cached entry price for the given position mint.
///
/// Returns `None` if no cache file exists or the file cannot be parsed.
pub fn load_entry_price(mint: &str) -> Option<f64> {
    let path = cache_path(mint).ok()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value.get("entry_price")?.as_f64()
}

/// Persist an entry price for the given position mint to the local cache.
///
/// Creates the cache directory if it does not yet exist.
pub fn save_entry_price(mint: &str, price: f64) -> Result<()> {
    let path = cache_path(mint)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::json!({ "entry_price": price });
    std::fs::write(&path, serde_json::to_string_pretty(&json)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_path_contains_mint() {
        let mint = solana_sdk::pubkey::Pubkey::new_unique().to_string();
        let path = cache_path(&mint).expect("valid pubkey");
        assert!(
            path.to_string_lossy().contains(&mint),
            "path should contain mint: {:?}",
            path
        );
        assert!(
            path.to_string_lossy().ends_with(".json"),
            "path should end with .json: {:?}",
            path
        );
    }

    #[test]
    fn test_cache_path_rejects_non_pubkey_mint() {
        assert!(cache_path("not-a-pubkey").is_err());
        assert!(cache_path("../../../etc/passwd").is_err());
        assert!(cache_path("").is_err());
    }

    #[test]
    fn test_load_entry_price_returns_none_for_missing_file() {
        let mint = solana_sdk::pubkey::Pubkey::new_unique().to_string();
        let result = load_entry_price(&mint);
        assert!(result.is_none());
    }

    #[test]
    fn test_load_entry_price_returns_none_for_invalid_mint() {
        assert!(load_entry_price("../../escape").is_none());
    }

    #[test]
    fn test_save_and_load_round_trip() {
        let mint = solana_sdk::pubkey::Pubkey::new_unique().to_string();
        let price = 142.5678;

        save_entry_price(&mint, price).expect("save should succeed");
        let loaded = load_entry_price(&mint).expect("load should return Some after save");
        assert!(
            (loaded - price).abs() < 1e-9,
            "loaded price should match saved price"
        );

        // Clean up
        let _ = std::fs::remove_file(cache_path(&mint).expect("valid pubkey"));
    }
}
