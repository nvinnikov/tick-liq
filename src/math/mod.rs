//! Pure CLMM math primitives.
//!
//! This module has **zero** Solana or protocol-crate dependencies. Everything
//! here operates on plain numeric types and can be tested in isolation.
//! Orchestration that requires Solana/protocol-specific conversions lives in
//! `crate::analytics`.

pub mod fees;
pub mod greeks;
pub mod il;
pub mod impact;
pub mod sqrt_price;
