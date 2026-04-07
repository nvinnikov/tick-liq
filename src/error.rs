//! Canonical error and result types for tick-liq.
//!
//! Per CLAUDE.md, code should use `anyhow` for error handling. This module
//! provides a single `Result` alias re-export so the rest of the crate can
//! `use crate::Result;` consistently.
//!
//! A thin `ConfigError` enum is also defined for the config loader, where
//! distinguishing missing-vs-malformed env input is useful for tests and
//! user-facing messages.

use thiserror::Error;

/// Crate-wide result alias. Prefer this over bare `anyhow::Result` so that we
/// have a single canonical type to swap out later if needed.
#[allow(dead_code)]
pub type Result<T> = anyhow::Result<T>;

/// Errors produced by the configuration loader.
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required config field: {0}")]
    MissingField(&'static str),

    #[error("invalid value for {field}: {message}")]
    InvalidValue {
        field: &'static str,
        message: String,
    },

    #[error("failed to read config file {path}: {source}")]
    FileRead {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse config file {path}: {source}")]
    FileParse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
}
