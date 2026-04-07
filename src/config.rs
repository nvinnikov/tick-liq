//! Runtime configuration loader.
//!
//! Configuration is layered in this order (later sources override earlier):
//!   1. Optional TOML file (path passed to [`Config::load`]).
//!   2. Environment variables.
//!
//! Per CLAUDE.md, **keypair material (paths or seeds) is read from environment
//! variables only** — never from TOML — to avoid accidental check-ins.
//!
//! Required fields: `rpc_url`, `ws_url`, `db_url`.
//! Optional fields: `drift_api_url`, `log_level` (defaults to `"info"`).
//!
//! Env var names:
//!   - `SOLANA_RPC_URL`
//!   - `SOLANA_WS_URL`
//!   - `DATABASE_URL`
//!   - `DRIFT_API_URL`
//!   - `LOG_LEVEL`
//!   - `KEYPAIR_PATH`           (env-only)
//!   - `KEYPAIR_SEED`           (env-only)

use crate::error::ConfigError;
use serde::Deserialize;
use std::path::Path;

const ENV_RPC_URL: &str = "SOLANA_RPC_URL";
const ENV_WS_URL: &str = "SOLANA_WS_URL";
const ENV_DB_URL: &str = "DATABASE_URL";
const ENV_DRIFT_API_URL: &str = "DRIFT_API_URL";
const ENV_LOG_LEVEL: &str = "LOG_LEVEL";
const ENV_KEYPAIR_PATH: &str = "KEYPAIR_PATH";
const ENV_KEYPAIR_SEED: &str = "KEYPAIR_SEED";

/// Fully-resolved runtime configuration.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Config {
    pub rpc_url: String,
    pub ws_url: String,
    pub db_url: String,
    pub drift_api_url: Option<String>,
    pub log_level: String,
    /// Path to a Solana keypair JSON file. Sourced from `KEYPAIR_PATH` env only.
    pub keypair_path: Option<String>,
    /// Raw keypair seed (hex/base58). Sourced from `KEYPAIR_SEED` env only.
    pub keypair_seed: Option<String>,
}

/// Partial configuration as loaded from a TOML file. All fields optional so
/// that env vars can supply or override anything.
#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    rpc_url: Option<String>,
    ws_url: Option<String>,
    db_url: Option<String>,
    drift_api_url: Option<String>,
    log_level: Option<String>,
}

#[allow(dead_code)]
impl Config {
    /// Load config, optionally overlaying values from a TOML file.
    ///
    /// Reads environment variables from the process env. To test against a
    /// custom env map, use [`Config::from_sources`] directly.
    pub fn load(file_path: Option<&Path>) -> Result<Self, ConfigError> {
        let file_cfg = match file_path {
            Some(path) => Some(load_file(path)?),
            None => None,
        };
        let env_lookup = |k: &str| std::env::var(k).ok();
        Self::from_sources(file_cfg, env_lookup)
    }

    /// Build a `Config` from an explicit file config + env lookup function.
    /// Exposed for tests so we don't have to mutate process-wide env state.
    fn from_sources<F>(file_cfg: Option<FileConfig>, env: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let file = file_cfg.unwrap_or_default();

        let rpc_url = env(ENV_RPC_URL)
            .or(file.rpc_url)
            .ok_or(ConfigError::MissingField("rpc_url"))?;
        let ws_url = env(ENV_WS_URL)
            .or(file.ws_url)
            .ok_or(ConfigError::MissingField("ws_url"))?;
        let db_url = env(ENV_DB_URL)
            .or(file.db_url)
            .ok_or(ConfigError::MissingField("db_url"))?;

        let drift_api_url = env(ENV_DRIFT_API_URL).or(file.drift_api_url);
        let log_level = env(ENV_LOG_LEVEL)
            .or(file.log_level)
            .unwrap_or_else(|| "info".to_string());

        // Keypair material: env-only.
        let keypair_path = env(ENV_KEYPAIR_PATH);
        let keypair_seed = env(ENV_KEYPAIR_SEED);

        Ok(Config {
            rpc_url,
            ws_url,
            db_url,
            drift_api_url,
            log_level,
            keypair_path,
            keypair_seed,
        })
    }
}

#[allow(dead_code)]
fn load_file(path: &Path) -> Result<FileConfig, ConfigError> {
    let contents = std::fs::read_to_string(path).map_err(|e| ConfigError::FileRead {
        path: path.display().to_string(),
        source: e,
    })?;
    toml::from_str::<FileConfig>(&contents).map_err(|e| ConfigError::FileParse {
        path: path.display().to_string(),
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_from(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |k: &str| map.get(k).map(|s| s.to_string())
    }

    #[test]
    fn loads_from_env_only() {
        let env = env_from(HashMap::from([
            (ENV_RPC_URL, "https://rpc.example"),
            (ENV_WS_URL, "wss://ws.example"),
            (ENV_DB_URL, "postgres://localhost/x"),
        ]));
        let cfg = Config::from_sources(None, env).unwrap();
        assert_eq!(cfg.rpc_url, "https://rpc.example");
        assert_eq!(cfg.ws_url, "wss://ws.example");
        assert_eq!(cfg.db_url, "postgres://localhost/x");
        assert_eq!(cfg.log_level, "info");
        assert!(cfg.drift_api_url.is_none());
        assert!(cfg.keypair_path.is_none());
        assert!(cfg.keypair_seed.is_none());
    }

    #[test]
    fn env_overrides_file() {
        let file = FileConfig {
            rpc_url: Some("https://file-rpc".into()),
            ws_url: Some("wss://file-ws".into()),
            db_url: Some("postgres://file".into()),
            drift_api_url: Some("https://file-drift".into()),
            log_level: Some("warn".into()),
        };
        let env = env_from(HashMap::from([(ENV_RPC_URL, "https://env-rpc")]));
        let cfg = Config::from_sources(Some(file), env).unwrap();
        assert_eq!(cfg.rpc_url, "https://env-rpc");
        assert_eq!(cfg.ws_url, "wss://file-ws");
        assert_eq!(cfg.log_level, "warn");
        assert_eq!(cfg.drift_api_url.as_deref(), Some("https://file-drift"));
    }

    #[test]
    fn missing_required_field_returns_clear_error() {
        let env = env_from(HashMap::new());
        let err = Config::from_sources(None, env).unwrap_err();
        match err {
            ConfigError::MissingField(f) => assert_eq!(f, "rpc_url"),
            other => panic!("expected MissingField, got {other:?}"),
        }
    }

    #[test]
    fn missing_ws_url_reports_ws_url() {
        let env = env_from(HashMap::from([
            (ENV_RPC_URL, "https://rpc"),
            (ENV_DB_URL, "postgres://x"),
        ]));
        let err = Config::from_sources(None, env).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ws_url"), "got: {msg}");
    }

    #[test]
    fn missing_db_url_reports_db_url() {
        let env = env_from(HashMap::from([
            (ENV_RPC_URL, "https://rpc"),
            (ENV_WS_URL, "wss://ws"),
        ]));
        let err = Config::from_sources(None, env).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("db_url"), "got: {msg}");
    }

    #[test]
    fn keypair_material_only_from_env() {
        // File has no place to set keypair fields, so even a populated file
        // cannot produce keypair config; only env can.
        let file = FileConfig {
            rpc_url: Some("https://rpc".into()),
            ws_url: Some("wss://ws".into()),
            db_url: Some("postgres://x".into()),
            ..Default::default()
        };
        let env = env_from(HashMap::from([
            (ENV_KEYPAIR_PATH, "/secrets/key.json"),
            (ENV_KEYPAIR_SEED, "deadbeef"),
        ]));
        let cfg = Config::from_sources(Some(file), env).unwrap();
        assert_eq!(cfg.keypair_path.as_deref(), Some("/secrets/key.json"));
        assert_eq!(cfg.keypair_seed.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn loads_file_from_disk() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("tick-liq-config-test-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            r#"
rpc_url = "https://from-file"
ws_url = "wss://from-file"
db_url = "postgres://from-file"
log_level = "debug"
"#,
        )
        .unwrap();
        let file_cfg = load_file(&path).unwrap();
        assert_eq!(file_cfg.rpc_url.as_deref(), Some("https://from-file"));
        assert_eq!(file_cfg.log_level.as_deref(), Some("debug"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_returns_file_read_error() {
        let err = load_file(Path::new("/nonexistent/tick-liq-xyz.toml")).unwrap_err();
        assert!(matches!(err, ConfigError::FileRead { .. }));
    }
}
