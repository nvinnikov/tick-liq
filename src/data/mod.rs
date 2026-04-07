//! Data layer: Solana RPC client pool, WebSocket pool subscriptions,
//! Pyth and CEX price feeds.
//!
//! This layer is the only place that performs network I/O against external
//! data sources. Everything else consumes typed snapshots produced here.

pub mod prices;
pub mod rpc;
pub mod ws;
