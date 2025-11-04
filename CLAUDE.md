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

# Property-based tests
cargo test --test math_tests

# Lint
cargo clippy -- -D warnings

# Format
cargo fmt

# Run CLI
cargo run -- pool info --address <POOL_ADDRESS>
cargo run -- position monitor --mint <POSITION_MINT>
cargo run -- backtest --pool <ADDRESS> --days 30 --strategy rebalance
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

- `solana-client`, `solana-sdk` 1.18, `anchor-client` 0.29
- `tokio` (full), `sqlx` (postgres + timescaledb), `clap` v4 (derive)
- `anyhow`, `tracing`, `reqwest`, `tokio-tungstenite`
- Dev: `proptest`
