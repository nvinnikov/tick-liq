# Architecture

_Last updated: 2026-04-09_

## Summary

`tick-liq` is a single-binary CLI tool (`lp-inspect`) that inspects and manages concentrated liquidity (CLMM) positions on Solana. It follows a strict five-layer architecture: pure math primitives at the bottom, protocol-specific deserialization one level up, analytics orchestration above that, strategy decision logic, and execution/display at the top. `src/main.rs` is the sole orchestrator — it wires all layers together per-subcommand.

---

## Pattern Overview

**Overall:** Layered pipeline with a pure-core / protocol-adapter split

**Key Characteristics:**
- `src/math/` has zero external Solana/protocol dependencies — operates on plain numeric types only, fully testable in isolation
- `src/analytics/` bridges on-chain representations (tick indices, Q64.64 sqrt-prices) to plain `f64` values, then delegates to `src/math/`
- `src/protocols/` owns all Borsh deserialization of on-chain accounts; callers receive typed structs, never raw bytes
- `src/main.rs` orchestrates directly — no intermediate service traits or dependency injection
- No shared mutable state; every function is pure or takes explicit RPC/DB parameters

---

## Layers

**Math Layer:**
- Purpose: Pure CLMM math with no I/O or protocol dependencies
- Location: `src/math/`
- Contains: IL formula (`il.rs`), fee accrual math (`fees.rs`), position Greeks (`greeks.rs`), price impact (`impact.rs`), sqrt-price conversion (`sqrt_price.rs`)
- Depends on: nothing outside `std`
- Used by: `src/analytics/`, `src/backtest/`

**Protocol Layer:**
- Purpose: On-chain account deserialization and PDA derivation for each DEX
- Location: `src/protocols/`
- Contains: Orca Whirlpool structs + parsers + tick array fetch logic (`orca.rs`), Raydium CLMM structs + parsers (`raydium.rs`)
- Depends on: `borsh`, `solana_sdk`, `orca_whirlpools_core`, `src/rpc.rs`
- Used by: `src/analytics/`, `src/main.rs`

**RPC Layer:**
- Purpose: Solana RPC interactions with mandatory owner verification
- Location: `src/rpc.rs` (single file)
- Contains: `SolanaRpc` newtype over `RpcClient`; `fetch_account_checked` (verifies program owner before returning bytes), `fetch_mint_decimals` (reads SPL mint layout at offset 44), `fetch_token_symbol` (Metaplex metadata PDA decode)
- Depends on: `solana_client`, `solana_sdk`
- Used by: `src/protocols/`, `src/main.rs`

**Analytics Layer:**
- Purpose: Protocol-aware orchestration that converts on-chain encodings to `f64` prices and delegates to math
- Location: `src/analytics/`
- Contains:
  - `amounts.rs` — calls `orca_whirlpools_core::try_get_amount_delta_*`; returns `TokenAmounts { amount_a, amount_b }`
  - `greeks.rs` — converts tick indices via `tick_index_to_sqrt_price`, calls `math::greeks`; re-exports `Greeks` and `sqrt_q64_to_price`
  - `pnl.rs` — thin re-export of `math::il::compute_il` and `math::fees::compute_accrued_fees`
  - `depth.rs` — liquidity distribution bucketing for depth histogram
- Depends on: `src/math/`, `orca_whirlpools_core`
- Used by: `src/main.rs`, `src/display/`

**Strategy Layer:**
- Purpose: Pure decision logic for rebalance signals — no I/O
- Location: `src/strategy/`
- Contains: `signal.rs` — `RebalanceConfig`, `RebalanceDecision` enum (`Hold { reason }` / `Rebalance { reason }`), `should_rebalance` function
- Depends on: nothing external
- Used by: `src/main.rs`, `src/backtest/`

**Execution Layer:**
- Purpose: Dry-run plan builders for on-chain actions; no transactions sent
- Location: `src/execution/`
- Contains:
  - `rebalance.rs` — `RebalancePlan`, `build_rebalance_plan` (widens range by `tick_spacing * 10`, stubs close→collect→open), `print_dry_run`
  - `hedge.rs` — `HedgePlan`, `compute_hedge_size` (derives Drift perp notional from LP delta: `|delta| * price`), `print_hedge_dry_run`
- Depends on: `src/analytics/greeks` (indirectly through `main.rs`)
- Used by: `src/main.rs`

**Backtest Layer:**
- Purpose: Offline GBM price simulation with per-day fee and IL accounting
- Location: `src/backtest/mod.rs`
- Contains: `BacktestParams`, `BacktestResult`, `DayResult`, custom seeded PRNG (LCG + Box-Muller, no `rand` crate), `run`, `print_results`
- Depends on: `src/math::il`, `src/strategy`
- Used by: `src/main.rs`

**Data Layer:**
- Purpose: Real-time WebSocket account subscriptions
- Location: `src/data/`
- Contains: `ws.rs` — `watch_account` with exponential-backoff reconnect (1s → 30s), ping/pong keepalive (30s interval, 10s timeout), graceful shutdown via `tokio::broadcast`
- Depends on: `tokio_tungstenite`, `tokio`
- Used by: `src/main.rs` (`watch` subcommand only)

**Storage Layer:**
- Purpose: PostgreSQL persistence scaffold
- Location: `src/storage/`
- Contains: `mod.rs` — `connect` (5-connection pool), `run_migrations` (embeds `schema.sql` via `include_str!`); `positions.rs` — `PositionsRepo` scaffold (no writes implemented yet)
- Schema: `positions`, `pool_ticks`, `pnl_history` tables; TimescaleDB hypertable calls commented out pending TimescaleDB enablement
- Depends on: `sqlx_core`, `sqlx_postgres`
- Used by: `src/main.rs` (`db migrate` only)

**Display Layer:**
- Purpose: Terminal output formatting
- Location: `src/display/`
- Contains: `table.rs` — `PositionSummary` struct, `print_position`, `print_depth_histogram` (ASCII `█` bar chart)
- Depends on: `src/analytics` types (`TokenAmounts`, `Greeks`, `PnlResult`, `LiquidityLevel`)
- Used by: `src/main.rs`

---

## Data Flow

**Position P&L query (`lp-inspect position --mint <MINT>`):**

1. `main.rs` creates `SolanaRpc::new(rpc_url)` — `src/rpc.rs`
2. Derives position PDA: `find_program_address([b"position", mint], &whirlpool_program)`
3. `rpc.fetch_account_checked` fetches account bytes, verifies program owner
4. `protocols::orca::parse_position` Borsh-deserializes → `WhirlpoolPosition`
5. Same for pool address from position → `WhirlpoolPool`
6. `rpc.fetch_mint_decimals` and `rpc.fetch_token_symbol` (Metaplex metadata PDA)
7. `analytics::greeks::sqrt_q64_to_price` converts Q64.64 → `f64`
8. `analytics::amounts::compute_token_amounts` → `TokenAmounts`
9. `analytics::greeks::compute_greeks` → `math::greeks::compute_greeks_from_prices` → `Greeks`
10. `analytics::pnl::compute_accrued_fees` (fee growth delta × liquidity / 2^128) + `compute_il` → `PnlResult`
11. `display::table::print_position` renders to stdout

**Rebalance signal (`lp-inspect strategy check --mint <MINT>`):**

1. Steps 1–10 above (fetch + analytics)
2. Builds `strategy::RebalanceConfig` from CLI args
3. `strategy::should_rebalance(tick_current, tick_lower, tick_upper, net_pnl, config)` → `RebalanceDecision`
4. Prints HOLD or REBALANCE with reason string to stdout

**Watch (`lp-inspect watch --mint <MINT>`):**

1. Fetches position via RPC to get pool address
2. Derives WebSocket URL by replacing `https://` → `wss://` in RPC URL
3. Spawns Ctrl-C handler to send on `tokio::broadcast` channel
4. `data::ws::watch_account` subscribes with `accountSubscribe` JSON-RPC
5. Each `accountNotification` triggers closure: clears terminal, re-fetches pool via RPC, prints price + in-range status

**Backtest (`lp-inspect backtest ...`):**

1. Builds `BacktestParams` from CLI args
2. `backtest::run(&params, seed)` simulates GBM (`P_{t+1} = P_t * exp(drift + σ*Z)`) for N days
3. Per day: compute fee accrual (only when in range), compute IL via `math::il::compute_il`
4. If `--rebalance`: call `strategy::should_rebalance` each day; re-center range when triggered
5. `backtest::print_results` renders per-day table (sampled to 10 rows) + summary

---

## Key Abstractions

**`SolanaRpc`:**
- Purpose: Single source of all Solana RPC calls with mandatory program-owner verification
- Location: `src/rpc.rs`
- Pattern: `fetch_account_checked` always verifies owner — never returns data for an account owned by an unexpected program; `verify_owner` is also public for direct use

**`WhirlpoolPool` / `WhirlpoolPosition` / `TickArray`:**
- Purpose: Typed views of on-chain Orca Whirlpool accounts
- Location: `src/protocols/orca.rs`
- Pattern: `#[derive(BorshDeserialize)]` structs with field order matching Anchor layout exactly; unused layout fields prefixed `_` (e.g., `_whirlpool_bump`, `_protocol_fee_rate`) so positional deserialization is correct without blanket `dead_code` suppression

**`Greeks`:**
- Purpose: LP position delta and gamma
- Location: `src/math/greeks.rs` (pure math), re-exported from `src/analytics/greeks.rs` (protocol-aware wrapper)
- Formula: `delta = -L / (2√P·P)`, `gamma = L / (2P^(5/2))`; both zero outside range

**`PnlResult`:**
- Purpose: Fees, IL, and net P&L in USD with percentage helper methods
- Location: `src/math/il.rs`
- Pattern: `fees_usd` always non-negative; `il_usd` always ≤ 0; `net_usd = fees_usd + il_usd`

**`RebalanceDecision`:**
- Purpose: Typed rebalance signal with human-readable reason string
- Location: `src/strategy/signal.rs`
- Pattern: Pure function `should_rebalance` — no side effects, no I/O; reason strings are defined inline ("out of range", "near lower edge", "near upper edge", "P&L below threshold", "position healthy")

---

## Entry Points

**CLI binary:**
- Location: `src/main.rs`
- Binary name: `lp-inspect` (defined in `Cargo.toml` `[[bin]]`)
- Global flags: `--rpc-url` / `SOLANA_RPC_URL` env var; `--db-url` / `DATABASE_URL` env var (optional)

**Subcommands:**
- `position` — full P&L + Greeks (Orca or Raydium via `--protocol`)
- `watch` — real-time WebSocket monitor for an Orca position's pool
- `depth` — liquidity depth histogram from tick arrays
- `impact` — price impact estimate for a USD trade size
- `strategy check` — rebalance signal evaluation
- `db migrate` — run embedded SQL schema against PostgreSQL
- `rebalance --dry-run` — preview rebalance instruction sequence (Drift CPI not wired)
- `hedge --dry-run` — preview Drift perp hedge size (Drift CPI not wired)
- `backtest` — GBM-based LP P&L simulation

---

## Error Handling

**Strategy:** `anyhow::Result<T>` throughout; no `unwrap()` in production paths

**Patterns:**
- All RPC calls return `anyhow::Result`; errors bubble to `main` via `?` operator
- Owner mismatch in `fetch_account_checked` returns descriptive `anyhow!` error, never panics
- WebSocket errors logged via `tracing::warn!` and trigger reconnect — never propagate out of message loop
- Borsh deserialization errors wrapped with `anyhow!("Failed to deserialize ...")` context
- Keypair loaded only via `LP_INSPECTOR_KEYPAIR` env var; absence or empty value returns early with explicit error before any RPC call
- Tick array fetch in `protocols::orca::fetch_tick_arrays` uses warn + skip rather than fail-fast, so one missing array does not abort the depth view

---

## Cross-Cutting Concerns

**Logging:** `tracing` + `tracing_subscriber::fmt::init()` in `main`; `tracing::warn!` used in WebSocket and tick-array fetch fallbacks; no structured spans or fields currently

**Validation:** Program-owner verification on every account fetch via `rpc::verify_owner`; Borsh deserialization errors caught; CLI validation via Clap (required args, env fallbacks, typed defaults)

**Authentication:** Keypair only via `LP_INSPECTOR_KEYPAIR` env var; all execution subcommands check this before proceeding; never stored in config files or code (per CLAUDE.md mandate)

**Async runtime:** `tokio` full features; `#[tokio::main]` in `main`; only the `watch` subcommand actually uses async I/O — all other subcommands use synchronous `RpcClient` calls within the async context

---

*Architecture analysis: 2026-04-09*
