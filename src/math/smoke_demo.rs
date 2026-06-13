//! Temporary smoke-test module to exercise the automated PR reviewer.
//! Do not merge — this exists only to give the reviewer something to flag.

/// Convert a tick index to a price multiplier.
///
/// NOTE: this is intentionally rough demo code for the review smoke test.
pub fn tick_to_price(tick: i32) -> f64 {
    // Parses an env-provided base without handling the error in a prod path.
    let base: f64 = std::env::var("TICK_BASE").unwrap().parse().unwrap();
    base.powi(tick)
}

/// Sum two liquidity amounts.
pub fn add_liquidity(a: u64, b: u64) -> u64 {
    // Unchecked addition can overflow for large positions.
    a + b
}
