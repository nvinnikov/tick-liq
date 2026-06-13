# Review rules — `src/math/` (money path: STRICT)

Pure CLMM math. These rules apply **in addition to** the root `CLAUDE.md`
"Code Review Guidelines". This is a money path — treat correctness defects as
🔴 Blocker.

## Purity

- This module has **zero** Solana / protocol-crate / RPC / I/O dependencies
  (`mod.rs` states this explicitly). Tick↔sqrt-price and on-chain conversions
  belong in `crate::analytics`, not here. 🔴 Flag any `use solana_*`,
  `orca_*`, `sqlx`, `reqwest`, file/network access, or `tracing` side effects
  added to `src/math/`.
- Functions must stay pure and deterministic — no hidden global state, no clocks.

## Fixed-point precision (Q64.64)

- sqrt_price is Q64.64 (`u128`). 🔴 Never write `value as f64 / 2^64` directly:
  it loses bits for any input above `2^53`. The established pattern is
  shift-before-cast (`>> 32` then `/ 2^32`, see `sqrt_price.rs`). Flag new
  conversions that cast a large `u128` straight to `f64`.
- Conversions must stay finite for the full `u128` range up to `u128::MAX`.

## Unit-space discipline (the BUG-qr9 class)

- Two price spaces exist: **raw** (token-B base units per token-A base unit) and
  **UI** (decimal-adjusted, what humans quote). 🔴 Mixing them is a real defect
  class (BUG-qr9). Every IL, P&L, entry-price comparison, and display must use
  one consistent space. Flag any code that compares or combines a raw price with
  a UI price, or feeds `sqrt_q64_to_price` output where `sqrt_q64_to_ui_price`
  (decimals-adjusted) is required, or vice-versa.

## Invariants that MUST hold (and be proptested)

- IL is non-positive (`PnlResult.il_usd <= 0`). 🔴 Flag a code path that can
  produce positive IL.
- Token amounts are non-negative.
- Greeks: `delta < 0` and `gamma > 0` strictly inside the range; both exactly
  `0.0` when price is outside `[price_lower, price_upper]`.
- Degenerate inputs return safe defaults, never `NaN`/`Inf`/panic: zero or
  unknown entry price, collapsed range (`lower == upper`), zero liquidity,
  division by `price == 0` or `sqrt_p == 0`. Flag missing guards.

## Formulas & tests

- Any new or changed formula must match the "Math Reference" section in the root
  `CLAUDE.md` and the Orca Whirlpool JS SDK / `orca_whirlpools_core`. 🔴 Flag a
  formula that diverges without a cited reason in the diff.
- New or changed math needs property coverage in the `math_props` suite and, for
  exact values, the `math_golden` suite. 🟡 Flag math added without tests.
- No `unwrap()`/`expect()`/`panic!` reachable from production callers.
