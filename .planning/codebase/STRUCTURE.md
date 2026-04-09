# Codebase Structure

_Last updated: 2026-04-09_

## Summary

Single Rust binary crate (`lp-inspect`) with all source in `src/`. Modules map directly to architectural layers. Integration tests live in `tests/` with fixtures under `tests/fixtures/`. No `src/lib.rs` — all modules are declared in `src/main.rs` via `mod` statements.

---

## Directory Layout

```
tick-liq/
├── Cargo.toml              # package manifest; binary name = lp-inspect
├── Cargo.lock
├── CLAUDE.md               # project spec + AI guidance
├── src/
│   ├── main.rs             # CLI entry point; declares all modules; routes subcommands
│   ├── rpc.rs              # SolanaRpc — RPC client with owner verification
│   ├── analytics/
│   │   ├── mod.rs          # re-exports sub-modules
│   │   ├── amounts.rs      # compute_token_amounts (delegates to orca_whirlpools_core)
│   │   ├── depth.rs        # build_distribution, estimate_impact, LiquidityLevel
│   │   ├── greeks.rs       # compute_greeks (tick-index → price → math::greeks)
│   │   └── pnl.rs          # re-exports math::il and math::fees
│   ├── backtest/
│   │   └── mod.rs          # BacktestParams, run, print_results; built-in PRNG
│   ├── data/
│   │   ├── mod.rs          # re-exports ws
│   │   └── ws.rs           # watch_account — WebSocket with reconnect + keepalive
│   ├── display/
│   │   ├── mod.rs          # re-exports table
│   │   └── table.rs        # PositionSummary, print_position, print_depth_histogram
│   ├── execution/
│   │   ├── mod.rs          # re-exports hedge and rebalance
│   │   ├── hedge.rs        # HedgePlan, compute_hedge_size, print_hedge_dry_run
│   │   └── rebalance.rs    # RebalancePlan, build_rebalance_plan, print_dry_run
│   ├── math/
│   │   ├── mod.rs          # declares sub-modules (zero Solana deps)
│   │   ├── fees.rs         # compute_accrued_fees (Q128 fee growth → token units)
│   │   ├── greeks.rs       # compute_greeks_from_prices → Greeks { delta, gamma }
│   │   ├── il.rs           # compute_il, PnlResult
│   │   ├── impact.rs       # price impact estimation
│   │   └── sqrt_price.rs   # sqrt_q64_to_price (Q64.64 → f64)
│   ├── protocols/
│   │   ├── mod.rs          # re-exports orca and raydium
│   │   ├── orca.rs         # WhirlpoolPool, WhirlpoolPosition, TickArray, parse_*, fetch_tick_arrays
│   │   └── raydium.rs      # Raydium CLMM pool/position structs and parsers
│   ├── storage/
│   │   ├── mod.rs          # connect, run_migrations (embeds schema.sql)
│   │   ├── positions.rs    # PositionsRepo scaffold (no writes yet)
│   │   └── schema.sql      # positions, pool_ticks, pnl_history DDL
│   └── strategy/
│       ├── mod.rs          # re-exports signal::{should_rebalance, RebalanceConfig, RebalanceDecision}
│       └── signal.rs       # pure rebalance decision function
├── tests/
│   ├── math_golden.rs      # golden-value tests against known IL/fee outputs
│   ├── math_props.rs       # property-based tests (proptest) for math invariants
│   └── fixtures/
│       └── orca_vectors.json  # Orca SDK reference vectors for IL/amount validation
└── .planning/
    └── codebase/           # GSD mapper output (ARCHITECTURE.md, STACK.md, etc.)
```

---

## Directory Purposes

**`src/math/`:**
- Purpose: Pure numeric CLMM math — no Solana, no protocol crates
- Contains: IL, fee accrual, Greeks, price impact, sqrt-price conversion
- Key files: `src/math/il.rs`, `src/math/greeks.rs`, `src/math/fees.rs`
- Zero dependencies outside `std`; start here for new math features

**`src/analytics/`:**
- Purpose: Protocol-aware wrappers that decode on-chain encodings then call `src/math/`
- Contains: token amount computation (via `orca_whirlpools_core`), Greeks with tick→price conversion, depth bucketing
- Key files: `src/analytics/amounts.rs`, `src/analytics/greeks.rs`

**`src/protocols/`:**
- Purpose: All Borsh deserialization of on-chain accounts; protocol-specific PDA derivation
- Contains: Orca Whirlpool and Raydium CLMM account structs
- Key files: `src/protocols/orca.rs` (most complete), `src/protocols/raydium.rs`
- Rule: field order in structs must match on-chain Anchor layout exactly; unused fields prefixed `_`

**`src/rpc.rs`:**
- Purpose: Wraps `solana_client::RpcClient`; enforces program-owner check before returning account data
- Key rule: all callers must pass `expected_owner`; `fetch_account_checked` verifies or errors

**`src/strategy/`:**
- Purpose: Pure decision functions for rebalance signals
- Key file: `src/strategy/signal.rs` — `should_rebalance` function with `RebalanceConfig` + `RebalanceDecision`

**`src/execution/`:**
- Purpose: Dry-run plan builders (no transactions sent yet)
- Key files: `src/execution/rebalance.rs`, `src/execution/hedge.rs`
- Note: Drift CPI not yet wired; both subcommands require `--dry-run` flag

**`src/backtest/`:**
- Purpose: Offline LP simulation over synthetic GBM price path
- Key file: `src/backtest/mod.rs` — single file containing params, PRNG, simulation loop, display

**`src/data/`:**
- Purpose: Real-time Solana account subscriptions
- Key file: `src/data/ws.rs` — `watch_account` with reconnect + keepalive

**`src/storage/`:**
- Purpose: PostgreSQL persistence (scaffold only, writes not yet implemented)
- Key files: `src/storage/mod.rs`, `src/storage/schema.sql`

**`src/display/`:**
- Purpose: Terminal rendering
- Key file: `src/display/table.rs` — `print_position`, `print_depth_histogram`

**`tests/`:**
- Purpose: Integration and property-based tests (outside `src/`)
- Key files: `tests/math_props.rs` (proptest invariants), `tests/math_golden.rs` (reference values)
- Fixtures: `tests/fixtures/orca_vectors.json` (Orca JS SDK reference vectors)

---

## Key File Locations

**Entry Point:**
- `src/main.rs` — CLI binary; all `mod` declarations; all subcommand routing

**Math Primitives:**
- `src/math/il.rs` — `compute_il(entry, current, lower, upper) -> f64`
- `src/math/greeks.rs` — `compute_greeks_from_prices(liquidity, price, lower, upper) -> Greeks`
- `src/math/fees.rs` — `compute_accrued_fees(growth_global, growth_checkpoint, liquidity) -> u64`
- `src/math/sqrt_price.rs` — `sqrt_q64_to_price(q64: u128) -> f64`

**On-chain Deserialization:**
- `src/protocols/orca.rs` — `parse_pool`, `parse_position`, `fetch_tick_arrays`
- `src/protocols/raydium.rs` — `parse_pool`, `parse_position`

**Strategy:**
- `src/strategy/signal.rs` — `should_rebalance`

**Execution Stubs:**
- `src/execution/rebalance.rs` — `build_rebalance_plan`
- `src/execution/hedge.rs` — `compute_hedge_size`

**Storage Schema:**
- `src/storage/schema.sql` — embedded via `include_str!` in `src/storage/mod.rs`

---

## Naming Conventions

**Files:** `snake_case.rs` (e.g., `sqrt_price.rs`, `signal.rs`)

**Directories:** `snake_case/` (e.g., `src/analytics/`, `src/backtest/`)

**Types, structs, enums:** `PascalCase` (e.g., `WhirlpoolPool`, `RebalanceDecision`, `HedgePlan`)

**Functions and variables:** `snake_case` (e.g., `compute_il`, `build_rebalance_plan`)

**Constants:** `SCREAMING_SNAKE_CASE` (e.g., `WHIRLPOOL_PROGRAM_ID`, `TICK_ARRAY_SIZE`, `PING_INTERVAL`)

**Test functions:** `snake_case` prefixed `test_` (e.g., `test_il_zero_at_entry_price`)

**Layout-only struct fields (Borsh):** prefixed `_` (e.g., `_whirlpool_bump`, `_protocol_fee_rate`) to satisfy positional deserialization without suppressing `dead_code`

---

## Where to Add New Code

**New math formula (pure):**
- Implementation: `src/math/<formula_name>.rs`
- Declare in: `src/math/mod.rs` with `pub mod <formula_name>;`
- Tests: inline `#[cfg(test)]` module + `tests/math_props.rs` for property invariants

**New analytics wrapper (protocol-aware):**
- Implementation: `src/analytics/<name>.rs`
- Declare in: `src/analytics/mod.rs`
- Pattern: convert on-chain types → `f64`, delegate to `src/math/`

**New protocol support:**
- Implementation: `src/protocols/<protocol>.rs`
- Declare in: `src/protocols/mod.rs`
- Required: Borsh struct with exact on-chain field order; `parse_pool` and `parse_position` functions; owner verification via `rpc.rs`

**New CLI subcommand:**
- Add variant to `Commands` enum in `src/main.rs`
- Add match arm in `main()` that constructs `SolanaRpc` and calls appropriate layers
- If execution action: add `--dry-run` guard; require `LP_INSPECTOR_KEYPAIR` env var check

**New execution plan:**
- Implementation: `src/execution/<name>.rs`
- Declare in: `src/execution/mod.rs` with `pub use`
- Pattern: pure plan-building function + print function; no transactions sent

**New strategy signal:**
- Implementation: add to `src/strategy/signal.rs` or new file in `src/strategy/`
- Must be pure (no I/O); accept tick indices or prices as plain numeric params

**Database table:**
- Add DDL to `src/storage/schema.sql`
- Add repo struct in `src/storage/positions.rs` or new file under `src/storage/`

---

## Special Directories

**`.planning/codebase/`:**
- Purpose: GSD mapper output documents (STACK, ARCHITECTURE, STRUCTURE, CONVENTIONS, TESTING, CONCERNS, INTEGRATIONS)
- Generated: by GSD mapper agents
- Committed: Yes (tracked in git)

**`.claude/worktrees/`:**
- Purpose: Git worktrees for parallel agent execution
- Generated: by GSD agent tooling
- Committed: No (agent-specific, not tracked)

**`tests/fixtures/`:**
- Purpose: Reference data for golden-value and integration tests
- Key file: `orca_vectors.json` — Orca JS SDK computed reference outputs
- Committed: Yes

---

*Structure analysis: 2026-04-09*
