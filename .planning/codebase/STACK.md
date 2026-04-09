# Technology Stack
_Last updated: 2026-04-09_

## Summary

`tick-liq` is a pure Rust CLI tool (`lp-inspect`) for inspecting and managing concentrated liquidity positions on Solana. It targets Rust 2021 edition, uses Tokio for async I/O, and ships as a single binary. There is no web server, frontend, or config file — all configuration is via environment variables.

## Language & Runtime

- **Language:** Rust (edition 2021)
- **Binary target:** `lp-inspect` — entry point `src/main.rs`
- **Async runtime:** `tokio` 1.x (`features = ["full"]`)
- **Package manager:** Cargo; `Cargo.lock` committed (151 KB)

## Core Frameworks & Libraries

| Crate | Version | Purpose |
|-------|---------|---------|
| `solana-client` | 1.18 | `RpcClient` — JSON-RPC account fetches (`src/rpc.rs`) |
| `solana-sdk` | 1.18 | `Pubkey`, `find_program_address`, key types |
| `orca_whirlpools_core` | 2.x | `tick_index_to_sqrt_price`; reference CLMM math |
| `borsh` | 0.10 | `BorshDeserialize` for Anchor on-chain account structs |
| `serde` | 1.x (`derive`) | Derive macros for serializable types |
| `serde_json` | 1.x | WebSocket JSON frame parsing |
| `tokio` | 1.x (`full`) | Async runtime, timers, `broadcast::channel` |
| `tokio-tungstenite` | 0.21 | WebSocket client — Solana `accountSubscribe` (`src/data/ws.rs`) |
| `futures-util` | 0.3 | `StreamExt`, `SinkExt` for WS stream handling |
| `clap` | 4.x (`derive`, `env`) | Subcommand CLI; env var fallback on every arg |
| `anyhow` | 1.x | `Result<T>`, `anyhow!()`, `.context()`; no `unwrap()` in prod |
| `tracing` | 0.1 | Structured logging macros (`info!`, `warn!`) |
| `tracing-subscriber` | 0.3 (`env-filter`) | `fmt` subscriber; level via `RUST_LOG` |
| `chrono` | 0.4 | UTC timestamps in `watch` command output |
| `sqlx-core` | 0.8 (`_tls-native-tls`) | Connection pool and executor traits |
| `sqlx-postgres` | 0.8 (`chrono`) | `PgPool`, `PgPoolOptions` — PostgreSQL driver |

**Note:** `anchor-client`, `reqwest`, and `toml` appear in CLAUDE.md as planned dependencies but are NOT present in `Cargo.toml`. The actual lockfile reflects the table above.

## Dev Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `proptest` | 1.x | Property-based tests for math invariants (`tests/`) |

## Database

- **PostgreSQL** with **TimescaleDB** extension (extension calls commented out in `src/storage/schema.sql` — tables created without hypertables by default)
- Schema embedded via `include_str!("schema.sql")` in `src/storage/mod.rs`
- Tables: `positions`, `pool_ticks`, `pnl_history`
- Pool: `PgPoolOptions::new().max_connections(5)`

## Configuration

All configuration is via environment variables only — no config file or TOML:

| Variable | Required | Default | Purpose |
|----------|----------|---------|---------|
| `SOLANA_RPC_URL` | No | `https://api.devnet.solana.com` | Solana JSON-RPC endpoint |
| `DATABASE_URL` | For `db migrate` | — | Postgres connection string |
| `LP_INSPECTOR_KEYPAIR` | For `rebalance`/`hedge` | — | Base-58 private key |
| `RUST_LOG` | No | — | Tracing filter (e.g. `info`) |

## Build & Tooling

```bash
cargo build                    # debug build
cargo build --release          # production binary
cargo test                     # all tests including proptest
cargo test --test math_tests   # property-based tests only
cargo clippy -- -D warnings    # lint (must pass clean)
cargo fmt                      # format
cargo run -- <subcommand>      # run CLI
```

## Platform Requirements

- Rust stable toolchain, edition 2021
- Network access to a Solana RPC node (HTTP + WSS on same base URL)
- PostgreSQL + TimescaleDB instance for storage subcommands
- No OS-specific dependencies detected; Linux and macOS supported

---

*Stack analysis: 2026-04-09*
