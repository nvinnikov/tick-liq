---
phase: quick
plan: 260411-k7g
subsystem: cli
tags: [cli, watch, entry-price, cache, il]
dependency_graph:
  requires: []
  provides: [entry_price_override_flag]
  affects: [src/main.rs]
tech_stack:
  added: []
  patterns: [clap-optional-arg, cache-override-before-fallback]
key_files:
  modified: [src/main.rs]
decisions:
  - Validation placed before risk-limit block to fail fast on bad input
  - Override unconditionally writes cache before Bug 3 fallback so is_none() guard correctly skips pool-price write
metrics:
  duration: ~10 min
  completed: 2026-04-11
  tasks_completed: 1
  files_modified: 1
---

# Quick Task 260411-k7g: Add Optional --entry-price Flag to watch Subcommand Summary

**One-liner:** Added `--entry-price <USD>` optional CLI flag to `watch` that unconditionally overrides the cached entry price before the Bug 3 pool-price fallback.

## What Was Done

Added an optional `--entry-price <USD>` flag to the `watch` subcommand so operators can supply the known entry price when re-watching a position opened at a specific price, enabling accurate IL calculation from the first tick without waiting for the pool-price fallback to trigger.

### Changes in src/main.rs

1. **New field on `Commands::Watch` variant** (line ~92):
   ```rust
   /// Entry price override (USD). If provided, unconditionally saves this as the
   /// cached entry price instead of using the current pool price at watch start.
   #[arg(long)]
   entry_price: Option<f64>,
   ```

2. **Added `entry_price` to destructuring pattern** in the `Commands::Watch` match arm.

3. **Validation block** (before risk-limit validation):
   ```rust
   if let Some(ep) = entry_price {
       if *ep <= 0.0 {
           anyhow::bail!("--entry-price must be positive (got {})", ep);
       }
   }
   ```

4. **Unconditional cache override** (before Bug 3 block):
   ```rust
   if let Some(ep) = entry_price {
       cache::save_entry_price(mint, *ep)?;
       tracing::info!(entry_price = *ep, "entry price overridden via --entry-price flag");
   }
   ```

   The existing Bug 3 block checks `cache::load_entry_price(mint).is_none()` and correctly skips when the flag already populated the cache.

## Verification

- `cargo build` — succeeded
- `cargo run -- watch --help | grep entry-price` — shows `--entry-price <ENTRY_PRICE>` with correct description
- `cargo clippy -- -D warnings` — pre-existing unrelated error in `src/math/fees.rs` (operator precedence); not caused by this change

## Commits

| Hash | Message |
|------|---------|
| 0d0de6b | feat(quick-260411-k7g): add optional --entry-price flag to watch subcommand |

## Deviations from Plan

None — plan executed exactly as written.

## Known Stubs

None.

## Threat Flags

None — the new CLI arg follows the same trust boundary as existing args (local-only cache write, price is not sensitive, logged at info level consistent with existing code).

## Deferred Items

Pre-existing clippy error in `src/math/fees.rs:26` (`operator precedence` on `a_lo * b_lo >> 64`) exists independent of this change and was present before this task.

## Self-Check: PASSED

- [x] `src/main.rs` modified — confirmed
- [x] Commit `0d0de6b` exists — confirmed
- [x] `--entry-price` appears in `watch --help` — confirmed
