# CONCERNS
_Last updated: 2026-04-09_

## Summary

The codebase has a functional CLI with math, analytics, and read-only Solana integration implemented. The critical gap is that the execution layer (`rebalance`, `hedge`) is entirely dry-run with no real transaction construction or signing, the storage layer is a no-op scaffold, and Raydium support is incomplete. Several `unwrap()` calls exist in production paths in `src/rpc.rs` and `src/analytics/amounts.rs`, and the backtest engine uses constant-liquidity approximations that meaningfully understate IL for real positions.

---

## Technical Debt

### `unwrap()` calls in production RPC metadata parsing
- **Issue:** `src/rpc.rs` lines 97, 103, 191, 193, 196 call `.unwrap()` on byte-slice `try_into()` during Metaplex metadata parsing. A malformed or truncated metadata account causes a panic rather than a graceful error.
- **Files:** `src/rpc.rs:97`, `src/rpc.rs:103`, `src/rpc.rs:191`, `src/rpc.rs:193`, `src/rpc.rs:196`
- **Impact:** Any pool whose token has non-standard metadata (truncated, corrupted, or versioned differently) crashes the CLI instead of falling back to the address-prefix display.
- **Fix approach:** Replace `.try_into().unwrap()` with `try_into().map_err(|_| anyhow!(...))` and propagate via `?`.

### `unwrap()` calls in `analytics/amounts.rs` tests bleeding into public API
- **Issue:** `src/analytics/amounts.rs` lines 66, 79, 92, 99 use `.unwrap()` directly.
- **Files:** `src/analytics/amounts.rs:66`, `src/analytics/amounts.rs:79`, `src/analytics/amounts.rs:92`, `src/analytics/amounts.rs:99`
- **Impact:** Test-only on lines 99 (test body), but lines 66/79/92 appear to be in the public compute path — requires confirming exact call sites. If in production paths, edge-case tick inputs will panic.
- **Fix approach:** Convert to `?` propagation; test `.unwrap()` is acceptable only inside `#[cfg(test)]` blocks.

### Two `build_distribution` functions with different signatures (type mismatch)
- **Issue:** `src/math/impact.rs` exports `build_distribution(tick_liquidities: &[(i32, i64)], ...)` while `src/analytics/depth.rs` defines its own `build_distribution(tick_liquidities: &[(i32, i128)], ...)`. The math layer version is `#[allow(dead_code)]` and unused. The analytics layer version is what `main.rs` calls.
- **Files:** `src/math/impact.rs:26-62`, `src/analytics/depth.rs:19-76`
- **Impact:** Dead code in `src/math/impact.rs` that diverges from the live implementation creates confusion. The `i64` vs `i128` element type difference means the math layer version cannot correctly handle large `liquidity_net` deltas from Orca (which are `i128` on-chain).
- **Fix approach:** Remove or reconcile the math-layer `build_distribution`; the analytics-layer version with `i128` is correct.

### `panic!()` in test assertions using wrong pattern
- **Issue:** `src/strategy/signal.rs` lines 99, 109, 119, 133 use `panic!("expected Rebalance")` inside `else` branches of `if let` blocks in tests, rather than `assert!(matches!(...))` or a `match` with exhaustive arms.
- **Files:** `src/strategy/signal.rs:99`, `src/strategy/signal.rs:109`, `src/strategy/signal.rs:119`, `src/strategy/signal.rs:133`
- **Impact:** Tests themselves are fine (panics in tests are expected on failure), but the pattern is inconsistent with the rest of the test suite which uses `assert!(matches!(...))`. No production risk.
- **Fix approach:** Replace with `assert!(matches!(d, RebalanceDecision::Rebalance { .. }))` for consistency.

### `sqlx-core` / `sqlx-postgres` used directly instead of the `sqlx` facade
- **Issue:** `Cargo.toml` imports `sqlx-core = "0.8"` and `sqlx-postgres = "0.8"` directly, bypassing the standard `sqlx` crate. `src/storage/mod.rs` calls internal APIs (`sqlx_core::executor::Executor`, `sqlx_core::raw_sql::raw_sql`).
- **Files:** `Cargo.toml:42-43`, `src/storage/mod.rs:4-5`
- **Impact:** The `raw_sql` API is not part of `sqlx`'s stable public surface and may break on minor version bumps. Using the public `sqlx` crate would provide compile-time query checking via `sqlx::query!`.
- **Fix approach:** Replace with `sqlx = { version = "0.8", features = ["postgres", "runtime-tokio-native-tls", "chrono"] }` and use `sqlx::query!` macros.

### `anchor-client` in CLAUDE.md but absent from `Cargo.toml`
- **Issue:** `CLAUDE.md` lists `anchor-client 0.29` as a dependency, but it does not appear in `Cargo.toml`. The Drift CPI hedge is a documented goal that requires Anchor.
- **Files:** `CLAUDE.md`, `Cargo.toml`
- **Impact:** Any work on live Drift CPI execution requires adding `anchor-client` first, which introduces a large dependency tree and version-pinning challenges with Solana 1.18.
- **Fix approach:** Track this as a prerequisite task before the Drift execution phase; do not add until needed.

---

## Incomplete Implementations

### Execution layer: no real transaction construction
- **What's missing:** `src/execution/rebalance.rs` and `src/execution/hedge.rs` are entirely dry-run. `RebalancePlan` and `HedgePlan` hold counts and estimated CU but contain no Solana instruction data, no account keys, and no signing logic.
- **Files:** `src/execution/rebalance.rs`, `src/execution/hedge.rs`
- **Blocks:** Live rebalancing, live delta hedging via Drift — the two core automation features of the project.
- **Note in code:** `hedge.rs:6` — "Drift CPI — that wiring is deferred to a later task"; `rebalance.rs:3` — "No Solana RPC calls, no transaction construction, no signing."

### Raydium support is partial
- **What's missing:** The `position` command for Raydium (`src/main.rs:276-304`) prints only pool address, price, tick, and raw liquidity. It does not compute token amounts, fees, IL, Greeks, or the formatted table that Orca receives.
- **Files:** `src/main.rs:276-304`, `src/protocols/raydium.rs`
- **Blocks:** Raydium position monitoring is unusable for P&L tracking.
- **Missing in `RaydiumPosition`:** `_token_fees_owed_0` / `_token_fees_owed_1` are prefixed `_` (layout-only) but are actually needed for fee display — the borsh struct includes them but marks them dead.
- **Note in README:** "Raydium support is partial — pool address, price, tick, liquidity only."

### Storage layer is a no-op scaffold
- **What's missing:** `src/storage/positions.rs` defines `PositionsRepo` with only `new()` and `pool()` — no insert, query, or update methods. The comment on line 5 says "Scaffold only — no writes yet." The schema defines `positions`, `pool_ticks`, and `pnl_history` tables but neither the CLI nor any code path writes to them.
- **Files:** `src/storage/positions.rs:5`, `src/storage/mod.rs`
- **Blocks:** P&L history persistence, position tracking across sessions, analytics on historical data.
- **TimescaleDB hypertable creation is commented out:** `schema.sql` lines 21 and 32 have `-- SELECT create_hypertable(...)` commented out, meaning `db migrate` creates plain PostgreSQL tables rather than TimescaleDB hypertables.

### `watch` command uses Orca only, no rebalance trigger
- **What's missing:** The `watch` command (`src/main.rs:306-383`) subscribes to the pool account via WebSocket and reprints price/tick/in-range status. It does not call `should_rebalance()`, does not log to storage, and does not trigger any execution path when out-of-range is detected.
- **Files:** `src/main.rs:306-383`
- **Blocks:** Automated rebalancing requires wiring the watch loop to the strategy and execution layers.

### Backtest uses constant-liquidity approximation for IL
- **What's missing:** `src/backtest/mod.rs:168` — `let position_value = params.initial_value_usd; // constant-liquidity approximation`. IL is computed using a fixed notional rather than the dynamically shrinking position value as the price drifts out of range. This understates IL for long out-of-range periods.
- **Files:** `src/backtest/mod.rs:167-168`
- **Impact:** Backtest IL numbers are optimistic (less negative) for simulations with many out-of-range days.

### Backtest `position_volume_share` parameter has no CLI argument
- **What's missing:** `BacktestParams` has a `position_volume_share` field (`src/backtest/mod.rs:30`) but the CLI `Backtest` subcommand in `src/main.rs` has no `--position-volume-share` argument. The field is hardcoded to `0.05` nowhere — it will be whatever value is default-initialized, which for `f64` is `0.0`, resulting in zero fees.
- **Files:** `src/backtest/mod.rs:30`, `src/main.rs:636-663`
- **Impact:** Every backtest run reports `$0.00` in fees unless `position_volume_share` is explicitly set. Looking at the `BacktestParams` construction in main.rs, this field is absent from the struct literal, which means this is a compile-time omission that Rust may fill with a default — but `BacktestParams` does not derive `Default`. This is a likely bug.

---

## Risks

### f64 precision for Q64.64 CLMM math
- **Risk:** `src/math/sqrt_price.rs` converts Q64.64 fixed-point values to `f64` for all calculations. IEEE-754 double precision has 53-bit mantissa; Q64.64 has 128-bit precision. Rounding diverges from on-chain results for extreme prices (very small or very large tick indices).
- **Files:** `src/math/sqrt_price.rs`, `src/analytics/greeks.rs`
- **Current mitigation:** Golden vector tests in `tests/math_golden.rs` validate against Orca SDK for a fixed set of ticks.
- **Gap:** No proptest or golden-vector coverage for extreme ticks near `MIN_TICK` / `MAX_TICK`.

### On-chain struct layout for Raydium not verified against mainnet
- **Risk:** `src/protocols/raydium.rs:19-22` has an explicit comment: "IMPORTANT: Verify field order against the actual program source before testing on mainnet." The borsh deserialization is order-sensitive; a single layout divergence silently produces wrong values.
- **Files:** `src/protocols/raydium.rs:19-22`
- **Current mitigation:** None — there are no golden-vector tests for Raydium deserialization.

### Rebalance centering strategy widens range rather than re-centering
- **Risk:** `src/execution/rebalance.rs:26` widens the range by `tick_spacing * 10` on each side (i.e., `new_tick_lower = old_lower - widen`, `new_tick_upper = old_upper + widen`). This does not re-center around the current price — it merely expands the old range. After a large price move, the new range can still be skewed away from current price.
- **Files:** `src/execution/rebalance.rs:26-38`
- **Impact:** Dry-run only currently, but the centering logic is mathematically wrong for the stated goal.

### WebSocket backoff does not reset on successful connect
- **Risk:** `src/data/ws.rs` resets backoff only after returning `Shutdown`; `run_session` returning `Reconnect` after a successful but short-lived session does not reset `backoff`. The backoff variable doubles on each `Reconnect` regardless of whether the last session was long or short.
- **Files:** `src/data/ws.rs:43-53`
- **Impact:** After several flaky connections, reconnect delay hits `RECONNECT_MAX` (30s) and stays there, creating a 30-second blind spot window.
- **Fix approach:** Reset `backoff` to `RECONNECT_BASE` inside `SessionResult::Reconnect` when the session was alive for more than a threshold duration (e.g., >5s).

### Keypair validation in `rebalance` and `hedge` is superficial
- **Risk:** `src/main.rs:601-610` and `src/main.rs:668-677` check that `LP_INSPECTOR_KEYPAIR` is set and non-empty, but do not validate that it is a valid base58 keypair. A typo in the env var would pass this check and fail only when the (not yet implemented) signing code runs.
- **Files:** `src/main.rs:601-610`, `src/main.rs:668-677`
- **Impact:** Misleading "validation passed" for an invalid key; low risk currently because no signing is performed.

---

## TODOs / Known Issues

### `schema.sql`: TimescaleDB hypertable calls commented out
- Lines 21 and 32 in `src/storage/schema.sql` have `SELECT create_hypertable(...)` commented out. Running `db migrate` creates plain PostgreSQL tables, defeating the TimescaleDB time-series performance goal.
- **Files:** `src/storage/schema.sql:21`, `src/storage/schema.sql:32`
- **Fix:** Uncomment, or wrap in a conditional that gracefully skips if the extension is unavailable.

### `src/math/impact.rs` `build_distribution` and struct types are `#[allow(dead_code)]`
- `LiquidityLevel`, `PriceImpact`, and `build_distribution` in `src/math/impact.rs` carry `#[allow(dead_code)]`. The live code uses the `analytics::depth` layer versions. The math-layer function has a different signature and is superseded.
- **Files:** `src/math/impact.rs:5-61`
- **Fix:** Remove the dead `build_distribution` from the math layer; keep only `estimate_impact` and the two structs which are re-exported by `analytics::depth`.

### `let _ = step;` suppression in `backtest/mod.rs`
- `src/backtest/mod.rs:291` has `let _ = step;` to suppress a dead-code warning for the `step` variable computed on line 262 and never used (the sampling logic was refactored to `sample_days`).
- **Files:** `src/backtest/mod.rs:262`, `src/backtest/mod.rs:291`
- **Fix:** Remove the `step` variable entirely.

### `data/mod.rs` is nearly empty
- `src/data/mod.rs` contains only `pub mod ws;`. CLAUDE.md specifies the data layer should include Pyth/CEX price feeds and a Solana RPC connection pool. Neither is implemented.
- **Files:** `src/data/mod.rs`

### No price feed integration
- The project has no live price feed (Pyth, Pyth Network, CoinGecko, Birdeye, or CEX). IL and P&L USD values in the `position` command derive token-B price from the pool's own sqrt_price, which is circular — token B is assumed to be a stablecoin (USDC). For SOL/USDC pools this is correct, but for arbitrary token pairs it will produce wrong USD values.
- **Files:** `src/main.rs:236-248`

### No integration or end-to-end tests
- All tests are unit tests within source files or integration tests over pure math (`tests/math_golden.rs`, `tests/math_props.rs`). There are no tests that exercise the CLI, the RPC path, or the database path. The watch loop, the `position` command, and all execution stubs have zero test coverage beyond their internal unit tests.
- **Files:** `tests/`

### `backtest` `position_volume_share` missing from CLI and struct construction
- As noted above in Incomplete Implementations, `BacktestParams` struct construction in `src/main.rs:649-661` does not set `position_volume_share`. If the struct has no `Default` impl this is a compilation error waiting to surface; if it silently defaults to 0.0 fees will always be zero.
- **Files:** `src/main.rs:649-661`, `src/backtest/mod.rs:13-36`
- **Priority:** High — likely silent wrong output bug.

---

## Dependency Risks

| Crate | Risk |
|-------|------|
| `solana-client 1.18` / `solana-sdk 1.18` | Solana 2.x is released; 1.18 is approaching EOL. Many downstream crates have already moved to 2.x, creating dependency incompatibilities. |
| `borsh 0.10` | Borsh 1.x is the current stable release; 0.10 uses the old `BorshDeserialize` trait API. `orca_whirlpools_core = "2"` may internally use borsh 1.x, creating duplicate/incompatible borsh versions in the build graph. |
| `orca_whirlpools_core = "2"` | Pinned to major version 2; Orca v3 SDK changes broke compatibility. Monitor for program upgrades that change account layouts. |
| `sqlx-core 0.8` (internal crate) | Using the internal `sqlx-core` crate directly rather than the public `sqlx` facade means no compile-time query checking and potential breakage on any minor release. |
| `tokio-tungstenite 0.21` | Solana WebSocket endpoints have specific quirks around ping/pong frames; RPC providers (Helius, QuickNode) may behave differently from public endpoints. |
