# STACK.md — Technology Stack

## Language & Runtime

- **Language:** Rust (edition 2021)
- **Async runtime:** Tokio (full features)
- **Target:** Solana mainnet / devnet

## Core Frameworks & Libraries

| Crate | Version | Purpose |
|-------|---------|---------|
| `solana-client` | 1.18 | Solana RPC, WebSocket subscriptions |
| `solana-sdk` | 1.18 | Keypairs, transactions, pubkeys |
| `anchor-client` | 0.29 | Anchor CPI calls to Orca/Raydium programs |
| `tokio` | 1 (full) | Async runtime |
| `clap` | 4 (derive) | CLI argument parsing |
| `serde` / `serde_json` | 1 | Serialization |
| `toml` | 0.8 | Config file parsing |
| `sqlx` | 0.7 | Async PostgreSQL driver |
| `anyhow` | 1 | Error handling |
| `tracing` | 0.1 | Structured logging |
| `tracing-subscriber` | 0.3 | Log output formatting |
| `reqwest` | 0.11 (json) | HTTP client (Pyth, CEX APIs) |
| `tokio-tungstenite` | 0.21 | WebSocket client |

## Dev Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `proptest` | 1 | Property-based testing for math invariants |

## Database

- **PostgreSQL** with **TimescaleDB** extension
- Used for: position snapshots, tick-level P&L history, events
- Access via `sqlx` with `runtime-tokio` and `chrono` features

## Configuration

- Format: TOML (`config/config.toml`, example at `config/config.toml.example`)
- Deserialized into typed structs in `src/config.rs`
- Sensitive values (RPC URLs, keypaths) via environment variables only

## Build & Tooling

```bash
cargo build           # debug build
cargo build --release # production
cargo clippy -- -D warnings
cargo fmt
cargo test
```

## Planned Toolchain Constraints

- No `unwrap()` in non-test code — use `anyhow::Result` and `?` propagation
- Clippy must pass with `-D warnings` (enforced in CI)
- 80%+ test coverage target for `src/math/` module
