//! Strategy layer: IL calculation, fee tracking, P&L, range optimization,
//! rebalance signal generation, and backtesting.
//!
//! Pure logic over data-layer snapshots — no I/O. Outputs are signals and
//! decisions consumed by the execution layer.

pub mod backtest;
pub mod fees;
pub mod pnl;
pub mod range;
pub mod signal;
