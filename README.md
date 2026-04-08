# tick-liq

Automated LP (Liquidity Provider) manager for concentrated-liquidity pools on Solana. Monitors CLMM positions (Orca Whirlpools / Raydium CLMM), computes real-time P&L (fees minus impermanent loss), generates rebalance signals, and — when the execution layer is wired end-to-end — closes stale positions, collects fees, and redeploys into a fresh range, optionally with a Drift perp delta-hedge.

The repository ships two artifacts today:

1. **`lp-inspect`** — a read-only CLI that parses on-chain CLMM positions and prints P&L, Greeks, depth, and price-impact breakdowns. This is the user-facing binary.
2. **`tick-liq` library crate** — the layered engine underneath (math, data, strategy, execution, storage) that the full rebalancer will be built from. Most modules are complete and fully tested; the execution layer is in active integration.

See [`docs/architecture.md`](docs/architecture.md) for a layer-by-layer walkthrough of the library, and [`CLAUDE.md`](CLAUDE.md) for the long-term product vision.

## Architecture at a glance

```
                 +-------------------------+
                 |         lp-inspect      |   (binary: CLI)
                 +-------------------------+
                             |
                             v
+----------+   +----------+   +--------------+   +-------------+
|   math   |<--| strategy |<--|   execution  |-->|   storage   |
+----------+   +----------+   +--------------+   +-------------+
     ^              ^                ^                 ^
     |              |                |                 |
     |              +--------+-------+                 |
     |                       |                         |
     |                       v                         |
     |                 +----------+                    |
     +-----------------|   data   |--------------------+
                       +----------+
                             |
                             v
          Solana RPC / WebSocket / Pyth / CEX feeds
```

- **`math/`** — pure CLMM math (tick ↔ price, liquidity ↔ amounts, IL, Greeks). No external deps, fully property-tested.
- **`data/`** — Solana RPC pool, WebSocket pool-state subscription with reconnect, Pyth/CEX price feeds.
- **`strategy/`** — fee tracker, P&L engine, range optimizer, rebalance signal generator, backtest replayer. All pure state machines.
- **`execution/`** — rebalance engine (close → collect → reopen), transaction signer/submitter, Drift perp hedge (WIP).
- **`storage/`** — PostgreSQL + TimescaleDB for positions, ticks, P&L history, rebalance events.

## Prerequisites

- **Rust toolchain** via [rustup](https://rustup.rs/) (stable, edition 2021).
- **A Solana RPC URL.** A public endpoint works for read-only inspection; a private RPC (Helius, Triton, QuickNode, …) is recommended for the `watch` / `monitor` commands since public endpoints rate-limit WebSocket subscriptions hard.
- **(Optional) PostgreSQL + TimescaleDB** for the storage layer and its integration tests. A `docker run` recipe is below.

## Build and test

```bash
# Build everything (library + lp-inspect binary)
cargo build --release

# Run all tests (unit + property + integration, excluding DB-gated tests)
cargo test

# Run just the property tests for the math layer
cargo test --test tick_props --test liquidity_props --test il_props --test greeks_props

# Lint and format
cargo clippy -- -D warnings
cargo fmt --check
```

The `lp-inspect` binary is produced at `target/release/lp-inspect`.

The storage integration tests (`tests/storage_db.rs`) are gated on the `TICKLIQ_DATABASE_URL` env var — they are skipped when it is unset, so `cargo test` is clean out of the box.

## Environment

| Variable                | Default                         | Purpose                                                                                            |
| ----------------------- | ------------------------------- | -------------------------------------------------------------------------------------------------- |
| `SOLANA_RPC_URL`        | `https://api.devnet.solana.com` | HTTP RPC endpoint used by `lp-inspect`. The `watch` / `monitor` subcommands derive the WS URL by swapping `https://` → `wss://`. |
| `TICKLIQ_DATABASE_URL`  | *(unset)*                       | PostgreSQL + TimescaleDB URL. Used by the storage layer and its integration tests.               |
| `TICKLIQ_CONFIG`        | `examples/config.toml.example`  | Path to a TOML config file consumed by `src/config.rs`. See the example for the full schema.      |

You can also pass `--rpc-url <URL>` on the command line to override `SOLANA_RPC_URL`.

## Running the CLI

All subcommands share the global `--rpc-url` flag:

```
lp-inspect [--rpc-url <URL>] <COMMAND>
```

### `pool info` — pool metadata

```
lp-inspect pool info --address <POOL>
```

Prints the current price, tick, pool liquidity, and fee tier for an Orca Whirlpool.

### `position` — full P&L breakdown (one shot)

```
lp-inspect position <MINT> [--protocol orca|raydium]
```

For Orca, prints pool metadata, current vs. range price, in-range status, token amounts, accrued fees (USD), impermanent loss, net P&L, and Greeks. The Raydium path currently prints a smaller summary.

### `monitor` — polled P&L updates

```
lp-inspect monitor --mint <MINT> [--interval-secs N]
```

Polls the position's pool on a timer (default 10s) and prints price, in-range status, and the instantaneous IL fraction. Orca only.

### `watch` — live pool subscription

```
lp-inspect watch <MINT>
```

Like `monitor`, but pushed: opens a WebSocket `accountSubscribe` to the pool and re-prints on every on-chain update. Lower-latency than `monitor` but depends on a reliable WebSocket.

### `depth` — liquidity around the current price

```
lp-inspect depth <POOL>
```

Estimates the USD trade size needed to move price ±1%, ±2%, ±5%, using pool-level liquidity (no tick-array walk yet).

### `impact` — price impact of a trade

```
lp-inspect impact <POOL> --size <USD>
```

Estimates post-trade price and percentage impact for buying `<USD>` worth of token A from the pool.

### `backtest` — replay a strategy against history *(stub)*

```
lp-inspect backtest --pool <ADDR> --days <N> --strategy rebalance
```

Currently a structured TODO stub — historical tick ingestion is not yet wired into the binary, but the underlying engine (`src/strategy/backtest.rs`) is fully implemented and unit-tested. See the backtest section of [`docs/architecture.md`](docs/architecture.md) for how to drive it from code today.

## Local database (PostgreSQL + TimescaleDB)

The storage layer (positions, pool ticks, P&L history, rebalance events) lives in PostgreSQL with the TimescaleDB extension. Migrations are in [`migrations/`](migrations/) as paired `*.up.sql` / `*.down.sql` files. `pool_ticks` and `pnl_history` are TimescaleDB hypertables partitioned on `ts`.

```bash
# Spin up a local Postgres + Timescale
docker run -d --name tick-liq-db \
  -e POSTGRES_PASSWORD=tickliq \
  -e POSTGRES_DB=tickliq \
  -p 5432:5432 \
  timescale/timescaledb:latest-pg16

export TICKLIQ_DATABASE_URL=postgres://postgres:tickliq@localhost:5432/tickliq
export DATABASE_URL=$TICKLIQ_DATABASE_URL   # sqlx-cli reads DATABASE_URL

# Apply migrations
cargo install sqlx-cli --no-default-features --features native-tls,postgres
sqlx migrate run     # apply all up migrations
sqlx migrate revert  # roll back the most recent migration
```

With `TICKLIQ_DATABASE_URL` set, `cargo test` will also run the storage integration tests in `tests/storage_db.rs`.

## Where to start reading the code

If you are new to the repository, read the layers bottom-up — each layer is small and self-contained.

1. **`src/math/`** — pure functions, no Solana deps. `tick.rs` and `liquidity.rs` define the CLMM primitives; `il.rs` and `greeks.rs` build the P&L ingredients on top. Start here to get comfortable with the math.
2. **`src/strategy/`** — `pnl.rs` composes math into a per-tick snapshot; `range.rs` picks a redeploy range; `signal.rs` decides when to rebalance; `backtest.rs` glues them together into a replay loop. All are pure state machines with no I/O, so they read like plain Rust.
3. **`src/data/`** — `rpc.rs` (pooled RPC), `ws.rs` (WebSocket with reconnect), `prices.rs` (Pyth + CEX). These are the thin I/O seams the strategy layer sits behind.
4. **`src/execution/`** — `rebalance.rs` (close → collect → reopen state machine), `tx.rs` (signer/submitter), `hedge.rs` (Drift perp hedge, WIP).
5. **`src/storage/`** — Postgres persistence: positions, ticks, P&L history, rebalance events.
6. **`src/main.rs`** — the `lp-inspect` CLI. This is the least interesting file in the repo; it is a thin clap-derive shell on top of the library.

[`docs/architecture.md`](docs/architecture.md) covers each layer in more depth, including the key types, invariants, and how they compose.

## Contributing: install git hooks

The repo ships a pre-push hook that refuses to push if `Cargo.lock` is out of sync with `Cargo.toml` — this catches the "forgot to commit the updated lockfile" class of CI failure locally instead of 20 minutes later on GH Actions. Run once after cloning:

```bash
./scripts/install-hooks.sh
```

This sets `core.hooksPath=.githooks` in your local repo config. The hook is a no-op for pushes that don't touch `Cargo.{toml,lock}` or any `*.rs` file. To bypass in an emergency: `git push --no-verify` (not recommended — CI will reject the same thing).

## Roadmap

- [`CLAUDE.md`](CLAUDE.md) — long-term product vision.
- [`docs/superpowers/plans/2026-04-07-followup-tasks.md`](docs/superpowers/plans/2026-04-07-followup-tasks.md) — concrete near-term follow-ups.

## License

License TBD — no `LICENSE` file is currently present in the repo.
