---
phase: quick-260411-l5d
plan: 01
subsystem: cli
tags: [bugfix, watch, entry-price, il]
key-files:
  modified: [src/main.rs]
decisions:
  - Validated entry_price > 0 and saved to cache before Bug 3 fallback so IL computes correctly when flag is used
metrics:
  duration: ~5 minutes
  completed: 2026-04-11
  tasks_completed: 1
  files_modified: 1
---

# Quick Task 260411-l5d: Bug — entry-price ignored in watch loop IL

**One-liner:** Restored `--entry-price` CLI flag on `watch` command, saving it to cache before the Bug 3 pool-price fallback so IL computes against the operator-supplied price rather than always zero.

## What Was Done

Commit `fad7411` (reset peak_pnl fix) accidentally removed the `--entry-price` field from the `Watch` enum variant in `src/main.rs`. Without it, IL always computed as zero because the cached entry price equalled the current pool price (Bug 3 fallback ran unconditionally).

Three changes restored in `src/main.rs`:

1. Added `entry_price: Option<f64>` field with `#[arg(long)]` to the `Watch` enum variant.
2. Added `entry_price` to the `Commands::Watch { ... }` match arm destructure.
3. Added validation (reject non-positive values) and cache override block immediately before the existing Bug 3 fix block, so that when `--entry-price` is provided the cache is populated and the `is_none()` guard correctly skips the pool-price fallback.

## Verification

- `cargo build` compiled successfully (1 file changed, 19 insertions).
- Pre-existing clippy error in `src/math/fees.rs` (operator precedence) is out of scope — confirmed present on baseline commit before this change.

## Deviations from Plan

None — plan executed exactly as written.

## Known Stubs

None.

## Threat Flags

None — no new network endpoints, auth paths, or schema changes introduced.

## Self-Check: PASSED

- `src/main.rs` modified: confirmed (git diff shows 19 insertions).
- Commit `0f57e7c` exists: confirmed via `git log`.
