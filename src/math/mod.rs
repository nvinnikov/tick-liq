//! Pure CLMM math: tick↔price conversion, liquidity↔amounts, IL, LP Greeks.
//!
//! No external I/O. Every function in this layer must be deterministic and
//! validated against the Orca Whirlpool JS SDK reference implementation.
//! Invariants (non-negative amounts, non-positive IL, etc.) are enforced via
//! `proptest` property tests.

pub mod greeks;
pub mod il;
pub mod liquidity;
pub mod tick;
