# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Automated LP (Liquidity Provider) Manager for concentrated liquidity pools on Solana. Monitors CLMM positions (Orca Whirlpools / Raydium CLMM), calculates real-time P&L (fees earned minus impermanent loss), and executes automatic range rebalancing with optional delta hedging via perp exchanges.

## Commands

```bash
# Build
cargo build

# Run tests
cargo test

# Run a single test
cargo test <test_name>

# Property-based + golden math suites (integration test targets)
cargo test --test math_props
cargo test --test math_golden

# Lint
cargo clippy -- -D warnings

# Format
cargo fmt

# Run CLI (binary is `lp-inspect`)
cargo run -- position --mint <POSITION_MINT> [--protocol orca|raydium] [--entry-price <P>]
cargo run -- watch --mint <POSITION_MINT> [--live] [--telegram] [--cex-symbol SOLUSDC]
cargo run -- depth --pool <POOL_ADDRESS>
cargo run -- impact --pool <POOL_ADDRESS> --size <USD>
cargo run -- strategy check --mint <POSITION_MINT>
cargo run -- db migrate           # applies the embedded schema (idempotent)
cargo run -- backtest --entry-price 84 --price-lower 75 --price-upper 95 --days 30 --rebalance
cargo run -- backtest --pool <POOL_ADDRESS> --from 2026-01-01 --to 2026-01-15 \
    --entry-price 84 --price-lower 75 --price-upper 95 --decimals-a 9 --decimals-b 6  # DB replay
cargo run -- rebalance --mint <POSITION_MINT> --dry-run
cargo run -- hedge --mint <POSITION_MINT> --dry-run
```

## Architecture

Three-layer design:

- **Data Layer** (`src/data/`) — Solana RPC client, WebSocket pool state subscription, Pyth/CEX price feeds. Use connection pool for RPC, `tokio-tungstenite` with reconnect for WebSocket.
- **Strategy Layer** (`src/strategy/`) — IL calculator, fee tracker, range optimizer, rebalance signal generator.
- **Execution Layer** (`src/execution/`) — Rebalance engine (close → collect fees → open position), Drift Protocol perp hedging via Anchor CPI.
- **Storage** (`src/storage/`) — PostgreSQL + TimescaleDB for positions, ticks, P&L history.
- **Math** (`src/math/`) — Pure Rust CLMM math (tick↔price conversion, liquidity/amounts, IL, delta/gamma). Start here — no external deps, fully testable.

## Key Technical Notes

- Use `anyhow` for all error handling; no `unwrap()` in production paths.
- Solana account deserialization: use `borsh` or the protocol's own crate (e.g., `whirlpool` crate for Orca). Always verify program owner before deserializing.
- Math must be validated against the [Orca Whirlpool JS SDK](https://github.com/orca-so/whirlpools) as the reference implementation.
- Test math with `proptest` property-based tests — invariants (e.g., amounts are non-negative, IL is non-positive) must hold across the full input space.
- Keypairs only via environment variables, never in config files or code.

## Math Reference

CLMM position amounts given liquidity `L`, price `P`, range `[Pa, Pb]`:
- `P < Pa`: `x = L*(1/√Pa - 1/√Pb)`, `y = 0`
- `P > Pb`: `x = 0`, `y = L*(√Pb - √Pa)`
- `Pa ≤ P ≤ Pb`: `x = L*(1/√P - 1/√Pb)`, `y = L*(√P - √Pa)`

LP delta (when in range): `delta = -L / (2√P * P)` — negative means naturally short volatility (source of IL).

Real P&L = `fees_earned - impermanent_loss`

## Dependencies

- `solana-client` 4.0-beta, `solana-sdk` 4 (façade); `orca_whirlpools_core` 2 for tick↔sqrt-price math. No `anchor-client` — accounts are parsed directly with `borsh` (discriminator skipped, owner verified).
- `tokio` (full), `tokio-tungstenite` (native-tls), `sqlx-core`/`sqlx-postgres` 0.8 (Postgres + TimescaleDB), `clap` v4 (derive + env)
- `anyhow`, `thiserror`, `tracing`, `serde`/`serde_json`, `base64`, `chrono`
- `binance-sdk` 45 (spot) for the CEX price feed; `teloxide` 0.13 for the Telegram bot
- Dev: `proptest`

## Code Review Guidelines

These instructions govern the automated PR review (`.github/workflows/claude-review.yml`).

**What to look for.** For this codebase, weigh — in priority order:

1. **Money-handling correctness** — CLMM math errors, liquidity/amount over/underflow, sign errors in IL/delta, off-by-one in tick math, rounding that leaks value. Cross-check math against the formulas in the "Math Reference" section above and against the Orca Whirlpool JS SDK.
2. **Security** — keypair or secret leakage, unverified program ownership before account deserialization, unchecked RPC/feed data, missing slippage/approval guards in execution paths.
3. **Robustness** — `unwrap()`/`expect()`/`panic!` in production paths, swallowed errors, missing reconnect on WebSocket feeds.
4. **Performance** — blocking calls in async paths, unbounded retries, N+1 RPC calls, missing connection pooling.

**Severity.** Tag every finding with one of:
- 🔴 **Blocker** — correctness / security / money-loss / panic in a production path. Must be fixed before merge.
- 🟡 **Should-fix** — a real issue that is not merge-blocking.

Anything that would be a style/formatting nit: do not post it. `cargo fmt` and `cargo clippy -D warnings` already gate those in CI.

**Depth (the review "level").** Default is **BALANCED**: report real issues of medium-or-higher confidence; skip speculative concerns. To change rigor, edit the `Depth:` line in the workflow prompt:
- *Blockers-only* — post only 🔴 findings; fewest comments.
- *Balanced* (default) — 🔴 + 🟡, medium+ confidence.
- *Exhaustive* — broad coverage including lower-confidence/edge findings; more comments, more false positives.

**Style.** Be concise and actionable — state the problem, why it matters, and the fix. Post inline comments on the exact lines; use the single summary comment for a severity tally and cross-cutting notes. If nothing is worth raising, say so briefly — do not manufacture findings.
