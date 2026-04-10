---
phase: 05-live-execution
plan: "01"
subsystem: execution
tags: [orca, whirlpool, cpi, transaction-building, instruction-builder]
dependency_graph:
  requires: []
  provides: [OrcaExecutor, position_pda, WhirlpoolPool.token_vault_a/b]
  affects: [src/execution, src/protocols/orca.rs]
tech_stack:
  added: [whirlpool-cpi@0.1.4 (git), anchor-lang@0.29.0, anchor-spl@0.29.0, spl-associated-token-account@1.1.3, spl-token@3.5.0]
  patterns: [off-chain Anchor instruction building, PDA derivation, ATA derivation]
key_files:
  created: [src/execution/orca_executor.rs]
  modified: [Cargo.toml, Cargo.lock, src/execution/mod.rs, src/protocols/orca.rs, src/main.rs]
decisions:
  - open_position uses 10 accounts (not 8 as in plan frontmatter) — matches actual whirlpool-cpi context.rs and RESEARCH.md
  - spl-token added explicitly (pulled transitively by anchor-spl but not accessible without explicit dep)
  - dead_code lint suppressed at module level in orca_executor.rs (wired up in plan 05-02)
metrics:
  duration: ~35 minutes
  completed: 2026-04-10
  tasks_completed: 2
  files_changed: 6
---

# Phase 05 Plan 01: OrcaExecutor + Cargo Deps Summary

**One-liner:** Off-chain Orca Whirlpool instruction builder using whirlpool-cpi git@anchor/0.29.0, implementing 4-step rebalance sequence with verified account layouts.

## What Was Built

### Task 1: Cargo Dependencies
Added to `Cargo.toml`:
- `whirlpool-cpi` git dep (`anchor/0.29.0` branch, `cpi` feature) — locked to commit `3d6587a86ab2fabaf157655cc8442a1a3703f368`
- `anchor-lang = "=0.29.0"` (exact pin)
- `anchor-spl = "=0.29.0"` (exact pin)
- `spl-associated-token-account = "1.1"` (explicit, needed for ATA derivation)
- `spl-token = "3.5"` (added during Task 2 — see deviations)

No `[patch.crates-io]` was needed. The `whirlpool-cpi anchor/0.29.0` branch is compatible with `solana-sdk 1.18` without any solana-program version patching.

### Task 2: OrcaExecutor + position_pda

**`src/protocols/orca.rs`:**
- Added `position_pda(position_mint: &Pubkey) -> Pubkey` — seeds `[b"position", mint.as_ref()]` under Whirlpool program
- Renamed `_token_mint_a`, `_token_vault_a`, `_token_mint_b`, `_token_vault_b` to public names (removing `_` prefix)
- Updated `main.rs` references to use renamed field names (4 call sites)

**`src/execution/orca_executor.rs`** (new):
- `OrcaRebalanceParams` — parameter struct for future `execute_rebalance` call
- `OrcaExecutor::new(rpc_url, Arc<Keypair>)` — holds `RpcClient` + keypair + program_id
- `ix_update_fees_and_rewards` — 4 accounts, uses `whirlpool_cpi::instruction::UpdateFeesAndRewards {}.data()`
- `ix_collect_fees` — 9 accounts, derives ATAs via `spl_associated_token_account::get_associated_token_address`
- `ix_close_position` — 6 accounts
- `ix_open_position` — 10 accounts, generates fresh `Keypair::new()` for position_mint, returns `(Instruction, Keypair)`
- `submit_tx` / `submit_tx_with_extra_signer` — transaction submission helpers (wired in 05-02)
- `simulate_tx` — `RpcClient::simulate_transaction` for integration tests

**`src/execution/mod.rs`:**
- Added `pub mod orca_executor` and `#[allow(unused_imports)] pub use orca_executor::OrcaExecutor`

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Added `spl-token = "3.5"` explicit dep**
- **Found during:** Task 2 first build attempt
- **Issue:** `spl_token::ID` used in instruction builders but `spl-token` crate was only transitively pulled by `anchor-spl`, not directly accessible
- **Fix:** Added `spl-token = "3.5"` to `[dependencies]` in Cargo.toml
- **Files modified:** `Cargo.toml`, `Cargo.lock`
- **Commit:** 9711738

**2. [Rule 1 - Bug] Updated `main.rs` field references after rename**
- **Found during:** Task 2 immediately after renaming `_token_mint_a/b` fields
- **Issue:** `src/main.rs` had 4 references to `pool._token_mint_a` and `pool._token_mint_b` that broke compilation after the field rename
- **Fix:** Updated all 4 references to use `pool.token_mint_a` / `pool.token_mint_b`
- **Files modified:** `src/main.rs`
- **Commit:** 9711738

**3. [Rule 2 - API deviation] open_position uses 10 accounts, not 8**
- **Found during:** Task 2 implementation
- **Issue:** Plan frontmatter states `open_position` has 8 accounts. Actual `whirlpool-cpi` `context.rs` `OpenPosition` struct has 10: funder, owner, position, position_mint, position_token_account, whirlpool, token_program, system_program, rent, associated_token_program. RESEARCH.md also documents 10.
- **Fix:** Implemented with 10 accounts matching the actual on-chain program requirement. Unit test asserts `ix.accounts.len() == 10`.
- **Files modified:** `src/execution/orca_executor.rs`
- **Commit:** 9711738

## whirlpool-cpi Cargo.lock Pin

```
name = "whirlpool-cpi"
version = "0.1.4"
source = "git+https://github.com/orca-so/whirlpool-cpi?branch=anchor%2F0.29.0#3d6587a86ab2fabaf157655cc8442a1a3703f368"
```

Commit hash: `3d6587a86ab2fabaf157655cc8442a1a3703f368`

## API Notes (whirlpool-cpi actual vs expected)

| Item | Expected (RESEARCH.md) | Actual (crate source) |
|------|----------------------|----------------------|
| `instruction::ClosePosition {}` | struct with no fields | struct with no fields ✓ |
| `instruction::CollectFees {}` | struct with no fields | struct with no fields ✓ |
| `instruction::UpdateFeesAndRewards {}` | struct with no fields | struct with no fields ✓ |
| `instruction::OpenPosition { bumps, tick_lower_index, tick_upper_index }` | struct with bumps field | struct with bumps field ✓ |
| `state::OpenPositionBumps { position_bump }` | available | available ✓ |
| `ToAccountMetas` via accounts struct | could use accounts struct | used raw `Vec<AccountMeta>` instead (simpler, avoids trait bound issues off-chain) |
| `InstructionData::data()` | available | available ✓ |

Raw `Vec<AccountMeta>` was preferred over `accounts.to_account_metas(None)` as it avoids instantiating `AccountPlaceholder` account wrappers that aren't meaningful off-chain.

## Compilation Warnings

1. `solana-client v1.18.26` — future-incompatibility warning (pre-existing, not introduced by this plan)
2. `whirlpool-cpi` duplicate packages warning — harmless, the crate repo contains multiple version subdirectories; Cargo correctly selects the root crate

Both warnings were pre-existing or structural; no new warnings introduced by this plan's code.

## WhirlpoolPool Field Renames

| Old name | New name | Reason |
|----------|----------|--------|
| `_token_mint_a` | `token_mint_a` | Needed for ATA derivation in `ix_collect_fees` |
| `_token_vault_a` | `token_vault_a` | Needed as writable account in `ix_collect_fees` |
| `_token_mint_b` | `token_mint_b` | Needed for ATA derivation in `ix_collect_fees` |
| `_token_vault_b` | `token_vault_b` | Needed as writable account in `ix_collect_fees` |

All other `_`-prefixed fields in `WhirlpoolPool` were left unchanged (borsh positional layout fields).

## Verification Results

```
cargo build    → Finished (0 errors, 2 pre-existing warnings)
cargo test     → 127 passed, 0 failed, 8 ignored
cargo clippy   → Finished exit 0 (0 errors)
orca_executor  → 4/4 unit tests pass
```

## Known Stubs

None — all implemented methods build real instructions. The `submit_tx` / `submit_tx_with_extra_signer` methods are real implementations not stubs; they are wired up in plan 05-02.

## Threat Surface Scan

The new `src/execution/orca_executor.rs` introduces a network endpoint (Solana RPC submission via `send_and_confirm_transaction`) and key management surface (`Arc<Keypair>` held in struct). Both are in-scope in the plan's threat model (T-05-03: keypair in RAM; T-05-05: fresh mint Keypair). No new threat surface beyond what was modeled.

## Task Commits

| Task | Commit | Description |
|------|--------|-------------|
| Task 1 | 1b88568 | chore(05-01): add whirlpool-cpi + anchor 0.29 + anchor-spl 0.29 deps |
| Task 2 | 9711738 | feat(05-01): implement OrcaExecutor + position_pda helper |

## Self-Check: PASSED
