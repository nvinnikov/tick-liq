---
phase: quick-260411-qr9
plan: "01"
subsystem: watch-loop / IL math
tags: [bug-fix, il, decimal-scaling, watch-loop]
dependency_graph:
  requires: []
  provides: [correct-IL-in-watch-loop]
  affects: [src/main.rs, src/math/il.rs]
tech_stack:
  added: []
  patterns: [tdd-regression-test]
key_files:
  modified:
    - src/main.rs
    - src/math/il.rs
decisions:
  - Only price_lower/price_upper in the watch loop were missing scaling; price_current was already correct at line 770
  - Did not alter display output or range_pct calculations (not present in this scope)
metrics:
  duration: "~10 minutes"
  completed: "2026-04-11T16:23:13Z"
  tasks_completed: 1
  files_modified: 2
---

# Phase quick-260411-qr9 Plan 01: Apply decimal scaling to price_lower/price_upper in watch loop

**One-liner:** Fixed IL always returning ~0 in watch loop by applying the same `* 10^(9-6)` decimal scaling to `price_lower` and `price_upper` that `price_current` already had.

## What Was Done

### Task 1 — Apply decimal scaling to price_lower and price_upper in watch loop

**Bug (BUG-qr9):** In the watch loop (`src/main.rs` ~lines 833–838), `price_lower` and `price_upper` were computed via `sqrt_q64_to_price()` without the decimal-scaling factor. This gave raw values around `0.084–0.096`, while `entry_price` and `price_current` were decimal-scaled to dollars (~`84–96`). Inside `compute_il`, the clamp `price_entry.sqrt().clamp(pa, pb)` collapsed both entry and current sqrt-prices to the same boundary (the upper boundary `~0.31`), yielding `IL ≈ 0` regardless of price movement.

**Fix:** Added `* 10f64.powi(9 - 6)` to both `price_lower` and `price_upper` computations at lines 833–842 in `src/main.rs`, matching the same factor applied to `price_current` at line 770.

**TDD:**
- RED commit `d38190a`: Added `test_il_nonzero_with_scaled_range` to `src/math/il.rs` — documents that scaled range produces non-zero IL and that unscaled range collapses IL to ~0 (bug reproduction).
- GREEN commit `b4ccfac`: Applied the fix to `src/main.rs`; build passes, all IL tests pass.

## Commits

| Commit | Type | Description |
|--------|------|-------------|
| d38190a | test | Add regression test for BUG-qr9 unit mismatch in IL computation |
| b4ccfac | fix | Apply decimal scaling to price_lower/price_upper in watch loop |

## Deviations from Plan

None — plan executed exactly as written.

## Known Stubs

None.

## Threat Flags

None. This is a pure arithmetic fix in an existing code path with no new trust boundaries.

## Self-Check: PASSED

- `src/main.rs` modified with `* 10f64.powi(9 - 6)` on both price_lower and price_upper
- `src/math/il.rs` contains `test_il_nonzero_with_scaled_range`
- Commits d38190a and b4ccfac exist in git log
- `cargo build` succeeded with no new errors
- All IL tests pass (19 unit + 5 golden + 6 prop)
