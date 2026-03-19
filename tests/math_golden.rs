//! Golden reference tests for the analytics math functions (task F9).
//!
//! Loads `tests/fixtures/orca_vectors.json` and asserts each vector's
//! expected values are reproduced by the current implementation within
//! tolerance. See `docs/math-validation.md` for how the vectors were
//! derived and how to regenerate them.
//!
//! The crate is binary-only, so we pull analytics modules in via
//! `#[path]`, matching the pattern used in `tests/math_props.rs`.

#![allow(dead_code)]

// analytics/amounts.rs is self-contained (uses orca_whirlpools_core directly).
#[path = "../src/analytics/amounts.rs"]
mod amounts;

// math modules are pure — include directly to avoid crate:: resolution issues.
#[path = "../src/math/il.rs"]
mod math_il;

#[path = "../src/math/sqrt_price.rs"]
mod math_sqrt_price;

#[path = "../src/math/greeks.rs"]
mod math_greeks;

use orca_whirlpools_core::tick_index_to_sqrt_price;
use serde::Deserialize;
use std::path::PathBuf;

use amounts::compute_token_amounts;
use math_il::compute_il;

fn compute_greeks(
    liquidity: u128,
    sqrt_price_q64: u128,
    tick_lower: i32,
    tick_upper: i32,
) -> math_greeks::Greeks {
    let price = math_sqrt_price::sqrt_q64_to_price(sqrt_price_q64);
    let price_lower = math_sqrt_price::sqrt_q64_to_price(tick_index_to_sqrt_price(tick_lower));
    let price_upper = math_sqrt_price::sqrt_q64_to_price(tick_index_to_sqrt_price(tick_upper));
    math_greeks::compute_greeks_from_prices(liquidity, price, price_lower, price_upper)
}

#[derive(Debug, Deserialize)]
struct Fixtures {
    amounts_vectors: Vec<AmountsVector>,
    il_vectors: Vec<IlVector>,
    greeks_vectors: Vec<GreeksVector>,
}

#[derive(Debug, Deserialize)]
struct AmountsVector {
    description: String,
    liquidity: u128,
    tick_current: i32,
    tick_lower: i32,
    tick_upper: i32,
    expected_amount_a: u64,
    expected_amount_b: u64,
    tolerance_abs: u64,
}

#[derive(Debug, Deserialize)]
struct IlVector {
    description: String,
    price_entry: f64,
    price_current: f64,
    price_lower: f64,
    price_upper: f64,
    expected_il: f64,
    tolerance_abs: f64,
}

#[derive(Debug, Deserialize)]
struct GreeksVector {
    description: String,
    liquidity: u128,
    tick_current: i32,
    tick_lower: i32,
    tick_upper: i32,
    expected_delta: f64,
    expected_gamma: f64,
    tolerance_rel: f64,
}

fn load_fixtures() -> Fixtures {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("orca_vectors.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
}

#[test]
fn golden_amounts_vectors() {
    let fx = load_fixtures();
    assert!(!fx.amounts_vectors.is_empty(), "no amounts vectors loaded");

    let mut failures: Vec<String> = Vec::new();
    for v in &fx.amounts_vectors {
        let sqrt_price = tick_index_to_sqrt_price(v.tick_current);
        let got = match compute_token_amounts(v.liquidity, sqrt_price, v.tick_lower, v.tick_upper) {
            Ok(a) => a,
            Err(e) => {
                failures.push(format!(
                    "[{}] compute_token_amounts failed: {e}",
                    v.description
                ));
                continue;
            }
        };

        let diff_a = got.amount_a.abs_diff(v.expected_amount_a);
        let diff_b = got.amount_b.abs_diff(v.expected_amount_b);
        if diff_a > v.tolerance_abs {
            failures.push(format!(
                "[{}] amount_a={} expected={} diff={} tol={}",
                v.description, got.amount_a, v.expected_amount_a, diff_a, v.tolerance_abs
            ));
        }
        if diff_b > v.tolerance_abs {
            failures.push(format!(
                "[{}] amount_b={} expected={} diff={} tol={}",
                v.description, got.amount_b, v.expected_amount_b, diff_b, v.tolerance_abs
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "amounts golden failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn golden_il_vectors() {
    let fx = load_fixtures();
    assert!(!fx.il_vectors.is_empty(), "no il vectors loaded");

    for v in &fx.il_vectors {
        let got = compute_il(v.price_entry, v.price_current, v.price_lower, v.price_upper);
        let diff = (got - v.expected_il).abs();
        assert!(
            diff <= v.tolerance_abs,
            "[{}] il={} expected={} diff={} tol={}",
            v.description,
            got,
            v.expected_il,
            diff,
            v.tolerance_abs
        );
        // Universal invariant: IL <= 0 whenever entry price is known.
        if v.price_entry != 0.0 {
            assert!(
                got <= 1e-12,
                "[{}] IL must be non-positive, got {}",
                v.description,
                got
            );
        }
    }
}

#[test]
fn golden_greeks_vectors() {
    let fx = load_fixtures();
    assert!(!fx.greeks_vectors.is_empty(), "no greeks vectors loaded");

    for v in &fx.greeks_vectors {
        let sqrt_price = tick_index_to_sqrt_price(v.tick_current);
        let got = compute_greeks(v.liquidity, sqrt_price, v.tick_lower, v.tick_upper);

        check_close(
            &v.description,
            "delta",
            got.delta,
            v.expected_delta,
            v.tolerance_rel,
        );
        check_close(
            &v.description,
            "gamma",
            got.gamma,
            v.expected_gamma,
            v.tolerance_rel,
        );
    }
}

fn check_close(desc: &str, label: &str, got: f64, expected: f64, tol_rel: f64) {
    if expected == 0.0 {
        assert!(
            got.abs() <= tol_rel.max(1e-12),
            "[{desc}] {label}={got} expected=0 tol_rel={tol_rel}"
        );
        return;
    }
    let rel = ((got - expected) / expected).abs();
    assert!(
        rel <= tol_rel,
        "[{desc}] {label}={got} expected={expected} rel_diff={rel} tol_rel={tol_rel}"
    );
}
