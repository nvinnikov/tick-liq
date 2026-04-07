//! tick-liq — automated LP manager for Solana CLMM pools.
//!
//! Library crate exposing the layered architecture described in `CLAUDE.md`.
//! The `lp-inspect` binary (see `src/main.rs`) is a separate target and does
//! not yet depend on these modules; migration of the existing inspector code
//! into these layers will happen in follow-up tasks.

pub mod data;
pub mod execution;
pub mod math;
pub mod storage;
pub mod strategy;
