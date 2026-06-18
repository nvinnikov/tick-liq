# Math validation against Orca Whirlpool reference

This document describes how `tick-liq`'s analytics math is validated against the
Orca Whirlpool reference implementation (task F9).

## Overview

The analytics module (`src/analytics/`) implements three pieces of CLMM math:

| Function                                    | File                     | Validates              |
|----------------------------------------------|--------------------------|------------------------|
| `compute_token_amounts(L, sqrtP, tl, tu)`    | `src/analytics/amounts.rs` | Position token balances |
| `compute_il(pe, pc, pl, pu)`                 | `src/analytics/pnl.rs`   | Impermanent loss       |
| `compute_greeks(L, sqrtP_q64, tl, tu)`       | `src/analytics/greeks.rs`| Delta and gamma        |

Validation is layered:

1. **Unit tests** (`src/analytics/*.rs`) — invariants (sign, zero cases, monotonicity).
2. **Property tests** (`tests/math_props.rs`) — holds across the full input space via `proptest`.
3. **Golden vectors** (`tests/math_golden.rs` + `tests/fixtures/orca_vectors.json`) — this doc. Pinned expected
   outputs locked against regression.

## Reference source

`compute_token_amounts` delegates directly to `orca_whirlpools_core` (the official
Rust port of the Orca Whirlpool on-chain/SDK math) for tick → sqrt_price conversion
and the `try_get_amount_delta_{a,b}` integer helpers. The JS SDK at
<https://github.com/orca-so/whirlpools> uses the same underlying formulas — so the
Rust and JS implementations agree by construction. The golden vectors therefore
serve as a **regression lock** that catches any future drift (e.g. from an upgrade
of `orca_whirlpools_core`, a local refactor, or an inadvertent formula change)
rather than an independent second implementation.

`compute_il` and `compute_greeks` are hand-written in Rust against the formulas in
`CLAUDE.md` (see "Math Reference" section):

- IL compares the LP portfolio against holding the entry composition, both valued
  at the *current* price. For L normalized to 1 and sqrt-prices clamped to
  `[√Pa, √Pb]`: `x = 1/√P̂ − 1/√Pb`, `y = √P̂ − √Pa`, and
  `IL = (x1·P1 + y1)/(x0·P1 + y0) − 1`. The clamp freezes the *holdings* outside
  the range while valuation still uses the unclamped current price, so
  out-of-range IL keeps growing (the full-range `2√k/(1+k)` formula ignores
  range width and understates concentrated IL ~10x).
- Greeks: `delta = -L / (2·√P·P)`, `gamma = d(delta)/dP = 3·L / (4·P²·√P)`,
  zero outside range.

These are numerically simple enough to be validated by hand-computed invariant
vectors (see below).

## Vector structure (`tests/fixtures/orca_vectors.json`)

```jsonc
{
  "amounts_vectors": [
    {
      "description": "...",
      "liquidity": 1000000000,        // u128
      "tick_current": 0,              // i32, converted to sqrt_price via orca crate
      "tick_lower": -100,             // i32
      "tick_upper": 100,              // i32
      "expected_amount_a": 4987272,   // u64
      "expected_amount_b": 4987272,   // u64
      "tolerance_abs": 50             // u64, absolute difference in raw units
    }
  ],
  "il_vectors": [
    {
      "price_entry": 100.0,
      "price_current": 80.0,
      "price_lower": 80.0,
      "price_upper": 120.0,
      "expected_il": -0.063588933,
      "tolerance_abs": 1e-7
    }
  ],
  "greeks_vectors": [
    {
      "liquidity": 1000000,
      "tick_current": 0,
      "tick_lower": -100,
      "tick_upper": 100,
      "expected_delta": -500000.0,
      "expected_gamma": 750000.0,
      "tolerance_rel": 1e-9
    }
  ]
}
```

## Vectors in this fixture

### Amounts (7 vectors)

| ID | Description | Hand-verified invariant |
|----|-------------|-------------------------|
| A1 | Zero liquidity | Exact: both amounts = 0 |
| A2 | Below range, L=1e9, [-100, 100] | `amount_b == 0` exact; amount_a from integer Q64.64 math |
| A3 | Above range, L=1e9, [-100, 100] | `amount_a == 0` exact; amount_b from integer Q64.64 math |
| A4 | In-range symmetric at tick 0, [-100, 100] | `amount_a == amount_b` (√Pa · √Pb = 1 symmetry) |
| A5 | In-range symmetric at tick 0, [-10, 10] | same symmetry, tighter range |
| A6 | Price at lower bound | Branch: in-range; `amount_b == 0` exact |
| A7 | Price at upper bound | Branch: above-range (code uses `>=`); `amount_a == 0` exact |

The exact integer values (e.g. `9999541`) are the Q64.64 fixed-point output of
`orca_whirlpools_core::try_get_amount_delta_{a,b}` — pinned once from a passing
run. Tolerance (`tolerance_abs`) is set to 50 raw units to absorb any future
micro-changes in rounding behavior of the underlying crate while still catching
real bugs.

### IL (6 vectors)

| ID | Description | Expected |
|----|-------------|----------|
| I1 | Entry == current | Exactly 0 |
| I2 | Entry price unknown (0.0) | Exactly 0 (early return) |
| I3 | Drop to lower bound | `-0.063588933` (hand-derived below) |
| I4 | Rise to upper bound | `-0.043353495` |
| I5 | Price far below range → LP frozen all-A at the bound, HODL keeps its B leg | `-0.820482326` |
| I6 | Price far above range → LP frozen all-B at the bound, HODL keeps its A leg | `-0.794221075` |

Tolerance `1e-7` absolute. Note I5/I6 are *not* equal to I3/I4: only the
holdings are clamped at the range boundary; the valuation price keeps moving,
so out-of-range IL keeps deepening.

I3 derivation, for reference (L normalized to 1):
```
pa  = √80  = 8.944271909999159
pb  = √120 = 10.954451150103322
sp0 = √100 = 10.0                          (in [pa, pb], no clamp)
sp1 = √80  = 8.944271909999159             (clamped to pa)

Entry holdings:   x0 = 1/sp0 − 1/pb = 0.008712223651...
                  y0 = sp0 − pa     = 1.055728090000841
Current holdings: x1 = 1/sp1 − 1/pb = 0.020516343914...
                  y1 = sp1 − pa     = 0

V_HODL(80) = x0·80 + y0 = 1.752705982...   →  1.752761 with full precision
V_LP(80)   = x1·80 + y1 = 1.641307513...

IL = V_LP/V_HODL − 1 = -0.063588933
```

The test runner additionally asserts the invariant `IL ≤ 0` whenever the entry
price is known, providing a global sanity check across all vectors.

### Greeks (6 vectors)

All greeks vectors are centered on tick 0 because `sqrt_price_q64 = 1 << 64`
gives `price = 1.0` exactly, making expected values integer-clean:

| ID | L     | Range        | Expected δ     | Expected γ    | Invariant |
|----|-------|--------------|----------------|---------------|-----------|
| G1 | 1e6   | [-100, 100]  | -500000        | 750000        | In range, P=1 → δ=-L/2, γ=3L/4 |
| G2 | 1e6   | above        | 0              | 0             | Out of range |
| G3 | 1e6   | below        | 0              | 0             | Out of range |
| G4 | 0     | [-100, 100]  | 0              | 0             | Zero liquidity |
| G5 | 2e6   | [-100, 100]  | -1000000       | 1500000       | Linearity in L |
| G6 | 1e6   | [-1000, 1000]| -500000        | 750000        | Depends only on P and L, not on range width |

Tolerance `1e-9` relative is near f64 precision.

## Running the golden tests

```bash
cargo test --test math_golden
```

All three sub-tests (`golden_amounts_vectors`, `golden_il_vectors`,
`golden_greeks_vectors`) must pass.

## Regenerating the fixture

The fixture is intentionally static — it is the "golden" regression lock.
Regenerate **only** when:

1. The underlying `orca_whirlpools_core` dependency is upgraded and a legitimate
   numeric drift is expected (verify the drift is sane before pinning), or
2. A bug is fixed in `compute_il` / `compute_greeks` that changes outputs (in which
   case the affected vectors should be manually re-derived from first principles,
   not blindly copied from the new implementation), or
3. New vectors are being added.

To find fresh expected values for amounts vectors after a dependency bump, the
simplest approach is to temporarily loosen `tolerance_abs` to a large value, run
the test with a print statement for the actual values, then pin them and tighten
the tolerance again.

For IL/greeks vectors, recompute from the formulas in `CLAUDE.md` §Math Reference
by hand before pinning — **do not** pin values produced by the implementation
you are trying to validate.

## Discrepancies found

Two were found and fixed during PR #64 review; the vectors above are the
re-derived values:

1. **IL formula** — the original implementation applied the *full-range*
   Uniswap-v2 ratio `2√k/(1+k)` with the range used only for clamping, which
   understates concentrated IL roughly 10x in range (and far more out of
   range). The original I3–I6 fixture values were pinned from that
   implementation's own output (violating the regeneration rule above), and an
   earlier revision of this document derived I3 with a third, also-incorrect
   variant (`2√r/(1+r)` with `r = sp1/sp0`, giving `-0.00155402`).
2. **Gamma** — implemented as `L/(2·P^2.5)`, inconsistent with the module's own
   `delta = -L/(2·P^1.5)` whose derivative is `3L/(4·P^2.5)` (1.5x larger).

Both formulas and all affected vectors were re-derived from first principles
(see I3 derivation above). All 7 amounts / 6 IL / 6 greeks vectors match the
current implementation within the documented tolerances.
