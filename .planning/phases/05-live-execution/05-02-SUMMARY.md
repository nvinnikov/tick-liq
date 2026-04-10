---
phase: 05-live-execution
plan: "02"
subsystem: execution
tags: [orca, live-mode, keypair, wallet, rebalance, shadow-guard, drift-stub, integration-tests]
dependency_graph:
  requires: [OrcaExecutor (05-01), ShadowGuard (02-xx), build_rebalance_plan (phase 3), spawn_shadow_write (phase 2)]
  provides: [load_wallet_keypair, LIVE-01 4-step rebalance path, LIVE-03 startup keypair guard, drift hedge stub, simulateTransaction integration tests]
  affects: [src/main.rs, src/execution/orca_executor.rs, src/execution/hedge.rs, src/execution/shadow_guard.rs, tests/live_rebalance.rs]
tech_stack:
  added: []
  patterns: [startup env-var validation with process::exit(1), direct enum match replacing submit() indirection, 4-step sequential transaction chain with retry, fire-and-forget error row (spawn_shadow_write)]
key_files:
  created: [tests/live_rebalance.rs]
  modified: [src/main.rs, src/execution/orca_executor.rs, src/execution/hedge.rs, src/execution/shadow_guard.rs, src/execution/mod.rs]
decisions:
  - load_wallet_keypair() is a named free function (not inlined) so it is clearly referenceable and testable
  - ShadowGuard::submit() kept with #[allow(dead_code)] — replaced by direct enum match in Watch arm; test coverage preserved
  - lp_delta inlined in main.rs (analytics::greeks::lp_delta did not exist); formula matches CLAUDE.md spec
  - rpc_url_clone added as explicit clone before closure capture to provide OrcaExecutor with RPC URL
  - cargo fmt applied to all source files (many pre-existing fmt violations fixed as part of Task 3)
  - live_rebalance.rs uses tick_array_pda() for ta_lower/ta_upper rather than Pubkey::new_unique() — more realistic account derivation
metrics:
  duration: ~35 minutes
  completed: 2026-04-10
  tasks_completed: 3
  files_changed: 17
---

# Phase 05 Plan 02: Live Execution Wiring Summary

**One-liner:** Wired OrcaExecutor 4-step rebalance into watch loop Live branch with WALLET_KEYPAIR startup guard, open_position retry, failure row persistence, drift hedge stub, and 4 simulateTransaction integration tests.

## What Was Built

### Task 1: Keypair loader + OrcaExecutor wiring + Drift stub

**`src/main.rs` — load_wallet_keypair() (line 211):**
- Free function reading `WALLET_KEYPAIR` env var as JSON `[u8; 64]`
- Returns `Err` with clear message if absent or malformed
- Called at line 487 in Watch arm immediately after `guard` is assigned
- `std::process::exit(1)` on any failure (line 493)

**`src/main.rs` — wallet_keypair capture (line 575):**
- `wallet_keypair_clone = wallet_keypair.clone()` before the `on_notify` closure
- `rpc_url_clone = rpc_url.clone()` added alongside existing `*_clone` bindings

**`src/main.rs` — OrcaExecutor wired at line 774:**
- Replaces old `guard.submit(&plan_proxy)` with direct `match guard { Shadow => ..., Live => ... }`
- Shadow branch logs decision without submitting
- Live branch: computes lp_delta inline (formula from CLAUDE.md), calls `execution::log_hedge_stub()`, then runs 4-step sequence
- Step 1: `ix_update_fees_and_rewards` → `execute_update_fees_and_rewards`
- Step 2: `ix_collect_fees` → `execute_collect_fees`
- Step 3: `ix_close_position` → `execute_close_position`
- Step 4: `ix_open_position` → `execute_open_position`, with 1 retry after 2s on failure
- Failure path at line 858: `tracing::error!` at CRITICAL level + `spawn_shadow_write` with `error_flag=true`, `trigger_reason="live_rebalance_failed"`

**`src/execution/orca_executor.rs` — 4 public execute_* methods added:**
- `execute_update_fees_and_rewards(ix)` → `submit_tx(ix)`
- `execute_collect_fees(ix)` → `submit_tx(ix)`
- `execute_close_position(ix)` → `submit_tx(ix)`
- `execute_open_position(ix, new_mint)` → `submit_tx_with_extra_signer(ix, new_mint)`
- `submit_tx` / `submit_tx_with_extra_signer` remain private; `#[allow(dead_code)]` removed from them (now called via public wrappers)

**`src/execution/hedge.rs` — log_hedge_stub() added:**
```rust
pub fn log_hedge_stub(plan: &HedgePlan) {
    tracing::info!(size_usd, side, delta, "drift hedge size computed (not submitted — LIVE-02 deferred)");
}
```

**`src/execution/shadow_guard.rs` — dead_code suppressions:**
- `#[allow(dead_code)]` added to `ShadowGuardError` and `submit()` — both kept for test coverage

### Task 2: simulateTransaction integration tests

**`tests/live_rebalance.rs`** (new, 145 lines):
- `simulate_update_fees_and_rewards_ix` — builds ix, calls simulate_tx (accepts program-level Err)
- `simulate_collect_fees_ix` — builds ix, asserts 9 accounts, calls simulate_tx
- `simulate_close_position_ix` — builds ix, asserts 6 accounts, calls simulate_tx
- `simulate_open_position_ix` — builds ix, asserts 10 accounts, verifies new_mint is signer, calls simulate_tx
- All 4 tests are `#[ignore]`; `cargo test --test live_rebalance` reports: 4 ignored, 0 failed

### Task 3: Final validation + cargo fmt

- `cargo fmt` applied globally — 16 source files formatted (many pre-existing violations fixed)
- `cargo fmt --check` now passes with 0 diffs
- `cargo clippy -- -D warnings`: clean
- `cargo test`: 127 passed, 0 failed, 12 ignored across all test targets
- `cargo build --release`: clean

## Keypair Loading at Startup (LIVE-03)

Load sequence in Watch arm:
1. `guard` assigned (Shadow or Live)
2. `load_wallet_keypair()` called if `run_mode == RunMode::Live`
3. On failure: `tracing::error!` + `std::process::exit(1)` — process never reaches RPC connect
4. On success: `Some(Arc::new(kp))` stored as `wallet_keypair`
5. `wallet_keypair_clone = wallet_keypair.clone()` before closure — moved into `on_notify`

## rpc_url_clone Capture

`rpc_url_clone` was needed and added at line 563:
```rust
let rpc_url_clone = rpc_url.clone();
```
The original `rpc_url` is consumed by the `rpc_inner` construction inside the closure; `rpc_url_clone` is the separate binding moved into the closure for use by `OrcaExecutor::new(...)`.

## lp_delta Inline (analytics::greeks::lp_delta absent)

`analytics::greeks::lp_delta` did not exist. The formula was inlined per CLAUDE.md spec:
```rust
let sqrt_price_f64 = pool.sqrt_price as f64 / (1u128 << 64) as f64;
let lp_delta = -(pos.liquidity as f64) / (2.0 * sqrt_price_f64 * price_current);
```

## Changes to OrcaExecutor Public API

| Method | Visibility | Purpose |
|--------|-----------|---------|
| `execute_update_fees_and_rewards` | `pub` (new) | Wraps `submit_tx` for step 1 |
| `execute_collect_fees` | `pub` (new) | Wraps `submit_tx` for step 2 |
| `execute_close_position` | `pub` (new) | Wraps `submit_tx` for step 3 |
| `execute_open_position` | `pub` (new) | Wraps `submit_tx_with_extra_signer` for step 4 |
| `submit_tx` | `fn` (private, unchanged) | Signs + submits single-signer tx |
| `submit_tx_with_extra_signer` | `fn` (private, unchanged) | Signs + submits multi-signer tx |

The `#[allow(dead_code)]` on `submit_tx` and `submit_tx_with_extra_signer` was removed since they are now called by the public execute_* wrappers.

## lib.rs pub mod Declarations

`src/lib.rs` already exported `pub mod execution` and `pub mod protocols` from phase 05-01 — no changes needed for integration test imports.

## Clippy Suppressions Added

| Suppression | Location | Rationale |
|-------------|----------|-----------|
| `#[allow(dead_code)]` on `ShadowGuardError` | `shadow_guard.rs:9` | Enum kept for test coverage; no longer used in prod path |
| `#[allow(dead_code)]` on `submit()` | `shadow_guard.rs:28` | Method kept for test coverage; replaced by direct match in main.rs |

No other suppressions were needed.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing functionality] Added `#[allow(dead_code)]` to ShadowGuard::submit and ShadowGuardError**
- **Found during:** Task 1, first clippy run
- **Issue:** Replacing `guard.submit(&plan_proxy)` with direct `match guard { ... }` left `submit()` and `ShadowGuardError` unused. `cargo clippy -- -D warnings` failed on both.
- **Fix:** Added `#[allow(dead_code)]` to preserve test coverage without clippy failure
- **Files modified:** `src/execution/shadow_guard.rs`
- **Commit:** 6ae4299

**2. [Rule 2 - Missing functionality] Applied cargo fmt to all source files**
- **Found during:** Task 3, `cargo fmt --check`
- **Issue:** `cargo fmt --check` found violations in `main.rs`, `orca_executor.rs`, `live_rebalance.rs`, and many pre-existing files. CLAUDE.md requires `cargo fmt` compliance.
- **Fix:** Ran `cargo fmt` globally; committed all reformatted files
- **Files modified:** 16 source files
- **Commit:** 9edff8b

**3. [Rule 1 - Bug] lp_delta inlined (analytics::greeks::lp_delta absent)**
- **Found during:** Task 1 implementation
- **Issue:** Plan referenced `analytics::greeks::lp_delta(pos.liquidity, pool.sqrt_price)` which does not exist in the codebase
- **Fix:** Inlined the formula from CLAUDE.md math reference: `-(L) / (2 * sqrt_price_f64 * price_current)` with Q64.64 → float conversion
- **Files modified:** `src/main.rs`
- **Commit:** 6ae4299

## Verification Results

```
cargo test                       → 127 passed, 0 failed, 12 ignored (all targets)
cargo test --test live_rebalance → 0 passed, 0 failed, 4 ignored
cargo clippy -- -D warnings      → Finished, exit 0 (clean)
cargo fmt --check                → exit 0 (no diffs)
cargo build --release            → Finished, exit 0 (clean)
```

## Known Stubs

- `log_hedge_stub()` in `src/execution/hedge.rs` — logs Drift hedge size but does not submit any Drift CPI. This is intentional per plan: LIVE-02 (Drift CPI) is deferred to a future plan.

## Threat Surface Scan

No new threat surface beyond what is modeled in the plan's `<threat_model>`. The new code paths are:
- `load_wallet_keypair()` reads WALLET_KEYPAIR env var — covered by T-05-06 (Spoofing, mitigated via `Keypair::from_bytes` validation and `process::exit` on failure)
- `execute_open_position` with retry — covered by T-05-08 (Tampering, mitigated via fresh `Keypair::new()` on retry)
- `spawn_shadow_write` failure row — covered by T-05-09 (DoS, mitigated via fire-and-forget pattern)
- `tracing::info!` for success signature — covered by T-05-11 (Repudiation, mitigated)

## Task Commits

| Task | Commit | Description |
|------|--------|-------------|
| Task 1 | 6ae4299 | feat(05-02): wire OrcaExecutor into live branch + keypair loader + drift stub |
| Task 2 | 1e5007b | test(05-02): add simulateTransaction integration tests for OrcaExecutor |
| Task 3 | 9edff8b | chore(05-02): cargo fmt — format all source files for clippy compliance |

## Self-Check: PASSED
