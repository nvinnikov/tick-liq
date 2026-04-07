//! Execution layer: rebalance engine (close → collect fees → reopen),
//! transaction signing/submission, and Drift Protocol perp hedging.

pub mod hedge;
pub mod rebalance;
pub mod tx;
