# Coding Conventions
_Last updated: 2026-04-09_

## Summary
Rust 2021 edition binary crate (no `lib.rs`). Error handling is uniformly `anyhow` with `?` propagation. The codebase enforces a strict two-layer split: pure math functions in `src/math/` have zero Solana/protocol deps and are fully standalone; `src/analytics/` orchestrates protocol-aware conversion on top. A deliberate `_` prefix convention marks borsh layout-only struct fields to silence dead-code lint without broad `#[allow]` suppressions.

---

## Naming Patterns

**Files:**
- One module per file, filename matches the concept: `il.rs`, `sqrt_price.rs`, `rebalance.rs`, `hedge.rs`
- Module entry points use `mod.rs` with thin re-exports only (see `src/analytics/mod.rs`, `src/strategy/mod.rs`)

**Functions:**
- `snake_case` throughout
- Verb-first for value-producing functions: `compute_il`, `compute_greeks_from_prices`, `build_rebalance_plan`, `estimate_impact`, `fetch_account_checked`, `parse_position`
- Print-only functions use `print_*` prefix: `print_dry_run`, `print_results`, `print_hedge_dry_run`

**Variables:**
- `snake_case`; price variables: `price_current`, `price_lower`, `price_upper`, `price_entry`
- Math temporaries mirror the formula: `sp0`, `sp1`, `pa`, `pb`, `sqrt_p`, `l`
- Byte offsets in manual layout parsing: `pos`

**Types / Structs:**
- `PascalCase`: `BacktestParams`, `BacktestResult`, `DayResult`, `WhirlpoolPool`, `SolanaRpc`, `RebalancePlan`
- Enum variants carry `reason: String` for human-readable context: `RebalanceDecision::Hold { reason }`, `RebalanceDecision::Rebalance { reason }`

**Constants:**
- `SCREAMING_SNAKE_CASE`: `WHIRLPOOL_PROGRAM_ID`, `TICK_ARRAY_SIZE`, `MINT_DECIMALS_OFFSET`, `EPS`

**Borsh layout-only fields (project-specific convention):**
- Prefix with `_` to silence dead-code lint without a blanket `#[allow(dead_code)]`
- Documented inline at `src/protocols/orca.rs` lines 28–31:
  ```rust
  pub _whirlpools_config: Pubkey,
  pub _token_mint_a: Pubkey,    // used for decimals/symbol lookup
  pub _protocol_fee_rate: u16,
  ```

---

## Error Handling

**Rule:** Use `anyhow` for all error handling. No `unwrap()` in production paths. `expect()` is permitted only on developer-invariant violations (hardcoded constants that must never be malformed).

**Patterns observed in `src/rpc.rs`, `src/analytics/amounts.rs`, `src/main.rs`:**

```rust
// Propagation with context at API boundaries
let pubkey = Pubkey::from_str(address)
    .map_err(|e| anyhow!("Invalid address '{}': {}", address, e))?;

// Early-return validation
if !dry_run {
    anyhow::bail!("Only --dry-run is supported");
}

// Expect only for hardcoded constants (panics = developer bug, not user error)
Pubkey::from_str(WHIRLPOOL_PROGRAM_ID).expect("hardcoded WHIRLPOOL_PROGRAM_ID is valid")
```

**Error message convention:** Include the offending value in the message string: `"Account '{}' not found: {}"`, `"Invalid address '{}': {}"`.

**In math modules:** Fallible operations return `Result<T>` (e.g., `compute_token_amounts` returns `Result<TokenAmounts>`). Infallible pure math functions return `T` directly (e.g., `compute_il`, `compute_greeks_from_prices`).

---

## Module Design

**Two-layer math/analytics separation (enforced by import structure):**

| Layer | Path | Dependencies |
|-------|------|-------------|
| Pure math | `src/math/` | No Solana, no protocol crates |
| Analytics orchestration | `src/analytics/` | Wraps `math::*`, adds `orca_whirlpools_core` conversions |
| Execution/strategy | `src/execution/`, `src/strategy/` | Pure functions, no on-chain I/O |
| CLI + RPC | `src/main.rs`, `src/rpc.rs` | All Solana I/O lives here |

**`mod.rs` as thin pass-through:**
```rust
// src/analytics/pnl.rs — entire file:
pub use crate::math::fees::compute_accrued_fees;
pub use crate::math::il::{compute_il, PnlResult};

// src/strategy/mod.rs — entire file:
pub mod signal;
pub use signal::{should_rebalance, RebalanceConfig, RebalanceDecision};
```

**Struct fields are `pub` directly** on data-only types — no getters: `BacktestParams`, `Greeks`, `HedgePlan`, `RebalancePlan`.

---

## Documentation

**Module-level doc comment (`//!`) required at top of every file:**
```rust
//! Pure CLMM math primitives.
//!
//! This module has **zero** Solana or protocol-crate dependencies.
```

**Inline comments explain the _why_ and cite formulas:**
```rust
// Standard CLMM IL: V_LP/V_HODL = 2√k/(1+k) where k = P1/P0.
// In sqrt-price terms: = 2·sp1·sp0 / (sp0² + sp1²)

// delta = -L / (2 * sqrt(P) * P)
// gamma = L / (2 * P^(5/2))

// GBM step: P_{t+1} = P_t * exp(drift + vol * Z)
```

**Layout references** cite the on-chain reference repository URL directly above the struct.

---

## Code Style

**Formatting:** `cargo fmt` with default rustfmt settings; no `.rustfmt.toml` present.

**Linting:** `cargo clippy -- -D warnings` (all warnings as errors, per CLAUDE.md). No `.clippy.toml` — default rules only.

**`#[allow(...)]` usage:** Narrow and item-scoped only. Examples:
- `#[allow(dead_code)]` on individual structs/functions when borsh requires layout fields or stubs are scaffolded
- `#![allow(dead_code)]` at crate root of integration tests (where `#[path]` includes produce unused items)

---

## Function Design

**Pure functions are the default:**
- `compute_il`, `should_rebalance`, `build_rebalance_plan`, `compute_hedge_size`, `estimate_impact` are all pure (no side effects, no I/O)
- I/O is isolated in `print_*` functions and `src/main.rs` match arms

**Numeric guards:**
```rust
price.max(1e-6)                      // guard against degenerate GBM paths
if price_entry == 0.0 { return 0.0; }  // unknown entry price -> no IL
if self.initial_value_usd == 0.0 { return 0.0; }  // division guard
```

**Saturation over panic for integer arithmetic:**
```rust
let widen = tick_spacing.saturating_mul(10);
let new_tick_lower = tick_lower.saturating_sub(widen);
```

---

## Async and Logging

**Async:** `#[tokio::main]` in `src/main.rs` only. `tokio::spawn` used for the Ctrl+C shutdown task in the Watch command. Sync RPC calls (`solana-client`) are used directly without `spawn_blocking`.

**Logging:** `tracing` crate; subscriber initialized once in `main()` with `tracing_subscriber::fmt::init()`. `tracing::warn!` used for non-fatal RPC/parse errors in the WebSocket watch loop.

---

## Security (Solana-specific)

**Keypairs via env vars only** — never in config files or code:
```rust
let keypair_b58 = std::env::var("LP_INSPECTOR_KEYPAIR").map_err(|_| {
    anyhow!("LP_INSPECTOR_KEYPAIR env var not set ...")
})?;
```

**Program owner verification before deserialization** (mandated in CLAUDE.md, implemented in `src/rpc.rs`):
```rust
pub fn fetch_account_checked(&self, address: &str, expected_owner: &Pubkey) -> Result<Vec<u8>> {
    ...
    verify_owner(address, &account.owner, expected_owner)?;
    Ok(account.data)
}
```
