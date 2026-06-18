// Library entry-point for integration tests and future external consumers.
// The binary (src/main.rs) is the primary artifact; this file re-exports the
// modules that integration tests need so they can use `tick_liq::storage`.

pub mod analytics;
pub mod backtest;
pub mod bot;
pub mod cache;
pub mod data;
pub mod display;
pub mod execution;
pub mod math;
pub mod metrics;
pub mod protocols;
pub mod research;
pub mod rpc;
pub mod storage;
pub mod strategy;
