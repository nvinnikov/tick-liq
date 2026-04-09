# Testing Patterns
_Last updated: 2026-04-09_

## Summary
Testing coverage is concentrated on the pure math layer (`src/math/`) with three complementary strategies: inline unit tests in every math module, property-based tests using `proptest` in `tests/math_props.rs`, and golden reference tests against JSON fixtures in `tests/math_golden.rs`. Higher-level modules (strategy, execution, rpc) have inline unit tests for pure functions. Network-dependent code is not unit-tested. The crate is binary-only (no `lib.rs`), so integration tests include source files directly via `#[path]`.

---

## Test Framework

**Runner:** `cargo test` (standard Rust built-in)

**Property-based testing:** `proptest = "1"` (dev-dependency only, `Cargo.toml` line 46)

**Assertion library:** Standard `assert!`, `assert_eq!`, `prop_assert!`, `prop_assert_eq!`

**Run Commands:**
```bash
cargo test                          # all tests (unit + integration)
cargo test math                     # filter by name prefix
cargo test --test math_props        # run property tests only
cargo test --test math_golden       # run golden reference tests only
cargo test -- --nocapture           # show println output
```

---

## Test File Organization

**Unit tests:** Co-located in `#[cfg(test)]` blocks at the bottom of each source file. Present in:
- `src/math/il.rs` — 3 tests
- `src/math/greeks.rs` — 4 tests
- `src/math/sqrt_price.rs` — 1 test
- `src/math/fees.rs` — 2 tests
- `src/math/impact.rs` — 4 tests
- `src/analytics/amounts.rs` — 4 tests
- `src/analytics/greeks.rs` — 3 tests
- `src/strategy/signal.rs` — 6 tests
- `src/execution/rebalance.rs` — 3 tests
- `src/execution/hedge.rs` — 3 tests
- `src/rpc.rs` — 6 tests

**Integration tests (`tests/` directory):**
- `tests/math_props.rs` — 8 property-based invariant tests (256 cases each)
- `tests/math_golden.rs` — 3 golden reference test functions driven by `tests/fixtures/orca_vectors.json`

**Fixtures:**
- `tests/fixtures/orca_vectors.json` — JSON file with `amounts_vectors`, `il_vectors`, `greeks_vectors` arrays; each entry carries `description`, inputs, expected values, and a `tolerance_abs` or `tolerance_rel` field

---

## Unit Test Structure

**Pattern in every math module:**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_<what>_<expected_outcome>() {
        // Arrange
        let result = function_under_test(args);
        // Assert with message
        assert!(result.condition, "human-readable failure message");
    }
}
```

**Helper functions** are defined locally in the test module when needed:
```rust
// src/analytics/amounts.rs
fn sqrt_price_at_tick(tick: i32) -> u128 {
    tick_index_to_sqrt_price(tick)
}

// src/analytics/greeks.rs
fn q64_at_tick(tick: i32) -> u128 {
    let sqrt_p = (1.0001f64.powi(tick)).sqrt();
    (sqrt_p * (1u128 << 64) as f64) as u128
}

// src/strategy/signal.rs
fn cfg() -> RebalanceConfig {
    RebalanceConfig { rebalance_out_of_range: true, near_edge_ticks: 10, min_net_pnl_usd: 0.0 }
}
```

---

## Property-Based Testing (`tests/math_props.rs`)

**Configuration:** 256 cases per property (`ProptestConfig::with_cases(256)`)

**The `#[path]` workaround:** Because the crate is binary-only (no `lib.rs`), integration tests include source modules directly:
```rust
#![allow(dead_code)]

#[path = "../src/analytics/amounts.rs"]
mod amounts;

#[path = "../src/math/il.rs"]
mod pnl;
```
All future integration tests should follow this same pattern.

**Custom strategies defined at top of file:**
```rust
fn tick_pair() -> impl Strategy<Value = (i32, i32)> { ... }  // ordered (lower, upper)
fn liquidity() -> impl Strategy<Value = u128> { ... }        // 1..=10^12
```

**Tick domain:** `MIN_TICK = -100_000`, `MAX_TICK = 100_000` (tighter than Orca's full ±443636 to keep `u64` amounts from overflowing).

**The 8 properties tested:**

| Property | Module | Invariant |
|----------|--------|-----------|
| `token_amounts_non_negative` | `amounts` | `compute_token_amounts` always succeeds and returns `u64` |
| `below_range_only_token_a` | `amounts` | `amount_b == 0` when price < range |
| `above_range_only_token_b` | `amounts` | `amount_a == 0` when price > range |
| `il_non_positive` | `il` | `compute_il` always returns `<= EPS (1e-9)` |
| `il_zero_at_identity` | `il` | IL is `~0` when `price_entry == price_current` |
| `greeks_delta_non_positive_in_range` | `greeks` | `delta <= EPS` when price is strictly inside range |
| `impact_monotone_in_size` | `impact` | doubling `pct` does not decrease `usd_needed` |
| `impact_monotone_in_liquidity` | `impact` | deeper pool requires `>=` USD for same `%` move |

**Typical property test:**
```rust
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn il_non_positive(
        price_entry in 1e-6f64..1e9,
        price_current in 1e-6f64..1e9,
        plo in 1e-6f64..1e9,
        phi_mul in 1.0001f64..1e6,
    ) {
        let phi = plo * phi_mul;
        let il = compute_il(price_entry, price_current, plo, phi);
        prop_assert!(il <= EPS, "IL must be <= 0, got {}", il);
        prop_assert!(il.is_finite(), "IL must be finite");
    }
}
```

---

## Golden Reference Tests (`tests/math_golden.rs`)

**Purpose:** Validate against pre-computed expected values derived from the Orca Whirlpool JS SDK.

**Fixture loading:**
```rust
fn load_fixtures() -> Fixtures {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/orca_vectors.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
}
```

**Three test functions:**
- `golden_amounts_vectors` — collects all failures into a `Vec<String>` and asserts the vec is empty (batch reporting, not fail-fast)
- `golden_il_vectors` — asserts `|got - expected| <= tolerance_abs` per vector, plus universal IL `<= 0` invariant
- `golden_greeks_vectors` — uses relative tolerance via `check_close` helper

**`check_close` helper pattern (relative tolerance):**
```rust
fn check_close(desc: &str, label: &str, got: f64, expected: f64, tol_rel: f64) {
    if expected == 0.0 {
        assert!(got.abs() <= tol_rel.max(1e-12), ...);
        return;
    }
    let rel = ((got - expected) / expected).abs();
    assert!(rel <= tol_rel, ...);
}
```

**Fixture format (`tests/fixtures/orca_vectors.json`):**
```json
{
  "amounts_vectors": [
    { "description": "A1: zero liquidity -> both amounts zero",
      "liquidity": 0, "tick_current": 0, "tick_lower": -100, "tick_upper": 100,
      "expected_amount_a": 0, "expected_amount_b": 0, "tolerance_abs": 0 }
  ],
  "il_vectors": [ ... ],
  "greeks_vectors": [ ... ]
}
```

---

## Mocking

No mocking framework is used. The codebase avoids the need for mocks through architecture:
- Pure math functions take plain numeric inputs — no mocking needed
- `rpc.rs` tests exercise only the pure helper functions (`verify_owner`, offset constant, manual layout parsing) — no live RPC calls in tests
- Network-dependent paths (`fetch_account_checked`, `fetch_mint_decimals`) are tested with invalid inputs (error path only) or not tested at unit level

---

## Tolerance Conventions

**Absolute tolerance** (`tolerance_abs`): Used for integer `u64` amounts. Typical value: `50` raw units (sub-cent given real decimals).

**Relative tolerance** (`tolerance_rel`): Used for `f64` Greeks. Typically `0.001` (0.1%) to `0.01` (1%).

**Float epsilon** (`EPS = 1e-9`): Used in property tests for "effectively zero" comparisons.

**Near-zero IL identity:** `1e-12` is used for the strict IL-at-identity check (`il.abs() < 1e-12`).

---

## Coverage Assessment

| Module | Test Type | Coverage |
|--------|-----------|----------|
| `src/math/il.rs` | Unit + proptest + golden | High — all branches covered |
| `src/math/greeks.rs` | Unit + proptest + golden | High |
| `src/math/sqrt_price.rs` | Unit + proptest (indirectly) | Adequate |
| `src/math/fees.rs` | Unit | Adequate (happy path + zero-growth) |
| `src/math/impact.rs` | Unit + proptest | High |
| `src/analytics/amounts.rs` | Unit + proptest + golden | High |
| `src/analytics/greeks.rs` | Unit | Adequate |
| `src/strategy/signal.rs` | Unit (6 tests) | High — all branches covered |
| `src/execution/rebalance.rs` | Unit (3 tests) | Adequate — negative ticks, saturation |
| `src/execution/hedge.rs` | Unit (3 tests) | Adequate |
| `src/rpc.rs` | Unit (pure helpers only) | Partial — no live RPC tests |
| `src/backtest/mod.rs` | None | **Gap** — no tests |
| `src/data/ws.rs` | None | **Gap** — no tests |
| `src/protocols/orca.rs` | None | **Gap** — no deserialization tests |
| `src/protocols/raydium.rs` | None | **Gap** — no tests |
| `src/storage/` | None | **Gap** — scaffold only |
| `src/display/` | None | **Gap** — print-only, low priority |

---

## What NOT to Test

- External protocol behavior (Orca SDK, Drift) — trust their SDKs
- Network-dependent paths in unit tests — no mock RPC boundary exists yet
- `main.rs` CLI command wiring — test the underlying pure functions instead
- `print_*` display functions — terminal output formatting, low ROI
