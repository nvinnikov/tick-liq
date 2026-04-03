# TESTING.md — Testing Patterns

## Framework

- **Unit tests:** `cargo test` — standard Rust `#[test]` in `#[cfg(test)]` modules
- **Integration tests:** `tests/` directory (`math_tests.rs`, `strategy_tests.rs`)
- **Property-based tests:** `proptest` crate — critical for mathematical correctness

## Test Structure

Unit tests live alongside source in `#[cfg(test)]` blocks:

```rust
// src/math/il.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_il_zero_at_entry_price() {
        let il = concentrated_il(1.0, 0.9, 1.1, 1.0);
        assert!(il.abs() < 1e-10);
    }
}
```

Integration tests in `tests/`:

```rust
// tests/math_tests.rs
use tick_liq::math::clmm::tick_to_price;

proptest! {
    #[test]
    fn tick_price_roundtrip(tick in -443636i32..443636) {
        let price = tick_to_price(tick);
        let back = price_to_tick(price);
        prop_assert!((back - tick).abs() <= 1);
    }
}
```

## Running Tests

```bash
cargo test                          # all tests
cargo test math                     # tests matching "math"
cargo test --test math_tests        # specific integration test file
cargo test -- --nocapture           # show println output
```

## Property-Based Testing (proptest)

Required for all math in `src/math/`. Key invariants to verify:

- `tick_to_price` is monotonically increasing
- `tick_to_price` ↔ `price_to_tick` roundtrip within ±1 tick
- IL is always ≤ 0 (LPs always lose relative to hold)
- IL = 0 when current price == entry price
- Position amounts are non-negative
- `x = 0` when `P > Pb`, `y = 0` when `P < Pa`
- Delta is negative when price is in range

## Math Validation

Math results must be cross-validated against the [Orca Whirlpool JS SDK](https://github.com/orca-so/whirlpools). For critical formulas, include a comment citing the reference implementation or whitepaper section.

## Coverage Target

- `src/math/`: 100% coverage
- `src/strategy/`: 80%+ coverage (pure functions)
- `src/execution/`: dry-run tested; mainnet paths integration tested on devnet
- `src/data/`: mock RPC responses for unit tests; real RPC in integration tests

## What NOT to Test

- External protocol behavior (Orca, Drift) — trust their SDKs
- Network-dependent paths in unit tests — mock at the RPC boundary
- `main.rs` CLI wiring — test the underlying functions instead
