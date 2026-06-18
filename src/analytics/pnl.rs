//! Thin re-export layer. All logic lives in `crate::math::il` and
//! `crate::math::fees`.

pub use crate::math::fees::compute_accrued_fees;
pub use crate::math::il::{PnlResult, compute_il};
