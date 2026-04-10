---
phase: "04-slippage-guard"
plan: "04-03"
subsystem: "strategy/slippage, cli"
tags: [cli, slippage, integration-tests, validation]
dependency_graph:
  requires: [04-01, 04-02]
  provides: [SLIPPAGE-01, SLIPPAGE-02, SLIPPAGE-03]
  affects: [src/main.rs, tests/slippage_tests.rs]
tech_stack:
  added: []
  patterns: [clap-default-value-t, closure-copy-before-capture, integration-test-public-api]
key_files:
  created:
    - tests/slippage_tests.rs
  modified:
    - src/main.rs
decisions:
  - "Copied *max_slippage_bps to max_slippage_bps_val before closure to satisfy 'static bound"
  - "Integration tests use tick_liq::strategy::slippage::* (lib re-export, not #[path] hack)"
metrics:
  duration: "15 minutes"
  completed: "2026-04-10"
  tasks_completed: 2
  files_modified: 2
---

# Phase 04 Plan 03: CLI flag --max-slippage-bps and integration tests Summary

**One-liner:** CLI `--max-slippage-bps` flag wired to `SlippageConfig::max_bps` with startup validation and 7 integration tests proving threshold pass/abort/edge-case behavior.

## What Was Built

### Task 1: --max-slippage-bps CLI flag (src/main.rs)
- Added `max_slippage_bps: u32` field to the `Watch` variant with `#[arg(long, default_value_t = 50)]`
- Added startup validation: `anyhow::bail!` if value is 0 or > 10_000, before any async work
- Replaced `SlippageConfig::default()` inside watch loop with `SlippageConfig { max_bps: max_slippage_bps_val }`
- Copied CLI value to `max_slippage_bps_val: u32` before the `'static` closure to avoid borrow lifetime error

### Task 2: Integration tests (tests/slippage_tests.rs)
Created 7 integration tests via `tick_liq::strategy::slippage::*`:
1. `test_small_trade_large_pool_passes_default_threshold` — $25k vs 1T liquidity at 150 → Ok < 50 bps
2. `test_large_trade_tiny_pool_exceeds_threshold` — $100k vs 1_000 liquidity → Abort > 50 bps, threshold=50
3. `test_custom_threshold_changes_outcome` — same trade passes at 50 bps, aborts at 10 bps
4. `test_zero_liquidity_always_aborts` — liquidity=0 → Abort with infinite impact
5. `test_zero_trade_size_always_passes` — position_value=0 → Ok with ~0 bps
6. `test_impact_increases_with_trade_size` — $1k vs $50k on same pool; larger has strictly higher bps
7. `test_cli_default_is_50_bps` — asserts `SlippageConfig::default().max_bps == 50`

## Verification Results

```
cargo build       — Finished (0 errors)
cargo clippy      — 0 errors (future-incompat warning from solana-client is pre-existing, not from this plan)
cargo test --test slippage_tests
  test result: ok. 7 passed; 0 failed; 0 ignored
cargo test --lib strategy::slippage
  6 passed, 125 filtered out (04-01 unit tests intact)
```

## Commits

| Task | Hash | Message |
|------|------|---------|
| T01  | 16876be | feat(04-03): add --max-slippage-bps CLI flag to Watch command with startup validation |
| T02  | 83f240f | test(04-03): add slippage guard integration tests (7 tests) |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Lifetime error: max_slippage_bps reference cannot be captured in 'static closure**
- **Found during:** Task 1
- **Issue:** The `Commands::Watch` match arm destructures `max_slippage_bps` as `&u32`. The `on_notify` closure passed to `watch_account` must be `'static`, so it cannot capture a reference to data owned by `cli.command`.
- **Fix:** Added `let max_slippage_bps_val: u32 = *max_slippage_bps;` before the closure, then used `max_slippage_bps_val` (a `Copy` value) inside the closure instead of the reference.
- **Files modified:** src/main.rs
- **Commit:** 16876be

## Known Stubs

None. The CLI value flows directly to `check_slippage()` in the watch loop.

## Threat Flags

None. This plan adds a configuration parameter to an existing validation path; no new network endpoints, auth paths, or schema changes introduced.

## Self-Check: PASSED

- [x] tests/slippage_tests.rs exists at correct path
- [x] src/main.rs contains `max_slippage_bps: u32` in Watch variant
- [x] src/main.rs contains `#[arg(long, default_value_t = 50)]`
- [x] src/main.rs contains startup bail with `--max-slippage-bps must be between 1 and 10000`
- [x] src/main.rs contains `SlippageConfig { max_bps: max_slippage_bps_val }`
- [x] Commits 16876be and 83f240f exist in git log
- [x] All 7 integration tests pass
- [x] All 6 unit tests from 04-01 still pass
