# tick-liq

`lp-inspect` is a read-only CLI for inspecting concentrated-liquidity (CLMM) positions on Solana. Today it parses Orca Whirlpool positions (with partial Raydium CLMM support), prints a P&L / Greeks breakdown, watches a pool over WebSocket, and answers basic depth and price-impact questions against the on-chain pool state.

This binary is the first slice of a larger automated LP manager — see [`CLAUDE.md`](CLAUDE.md) for the broader vision (rebalancing engine, perp hedging, TimescaleDB-backed history).

## Prerequisites

- **Rust toolchain** via [rustup](https://rustup.rs/) (stable, edition 2021)
- **A Solana RPC URL.** A public endpoint works for read-only inspection; a private RPC (Helius, Triton, QuickNode, …) is recommended for `watch` since the public endpoints rate-limit WebSocket subscriptions hard.

## Build

```bash
cargo build --release
```

The binary is produced at `target/release/lp-inspect`.

## Environment

| Variable          | Default                              | Purpose                                                       |
| ----------------- | ------------------------------------ | ------------------------------------------------------------- |
| `SOLANA_RPC_URL`  | `https://api.devnet.solana.com`      | HTTP RPC endpoint. Must be reachable; `watch` derives the WS URL by swapping `https://` → `wss://`. |

You can also pass `--rpc-url <URL>` on the command line to override.

## Usage

All subcommands share the global `--rpc-url` flag:

```
lp-inspect [--rpc-url <URL>] <COMMAND>
```

### `position` — full P&L breakdown

```
lp-inspect position <MINT> [--protocol orca|raydium]
```

- `<MINT>` — position NFT mint address
- `--protocol` — `orca` (default) or `raydium`

For Orca, prints pool metadata, current vs. range price, in-range status, token amounts, accrued fees (USD), impermanent loss, net P&L, and Greeks. The Raydium path currently prints a smaller summary (pool, price, tick, liquidity).

```bash
lp-inspect --rpc-url "$SOLANA_RPC_URL" position 4xj... --protocol orca
```

Example output (Orca):

```
Pool:        Hp7...   fee 5 bps
Price:       $24.1873   range [$22.50, $26.00]   IN-RANGE (62%)
Amounts:     1.234 A   |   29.87 B
Fees:        $4.21    IL: -$0.83    Net: $3.38
Greeks:      delta=-0.0123  gamma=0.000041
```

### `watch` — live pool subscription

```
lp-inspect watch <MINT>
```

Resolves the position's pool, opens a WebSocket `accountSubscribe` to that pool, and re-prints price / tick / in-range status on every update. Orca only. Ctrl+C to stop.

```bash
lp-inspect --rpc-url "$SOLANA_RPC_URL" watch 4xj...
```

Example tick:

```
[14:02:11 UTC] Pool update received

Pool:      Hp7...
Price:     $24.1901
Tick:      -32184
In range:  YES
Liquidity: 1284732001
```

### `depth` — liquidity around the current price

```
lp-inspect depth <POOL>
```

Reads pool-level liquidity and estimates the USD trade size needed to move price ±1%, ±2%, ±5%. Orca pools only.

```bash
lp-inspect --rpc-url "$SOLANA_RPC_URL" depth Hp7...
```

Example output:

```
Liquidity Distribution  (pool liquidity: 1284M)
──────────────────────────────────────────────────
  +1%  (~$24.4292): $4123 needed to buy  |  $4087 needed to sell
  +2%  (~$24.6710): $8210 needed to buy  |  $8113 needed to sell
  +5%  (~$25.3967): $20114 needed to buy  | $19782 needed to sell

Note: uses pool-level liquidity. Tick-array depth map coming in a future update.
```

### `impact` — price impact of a specific trade

```
lp-inspect impact <POOL> --size <USD>
```

Estimates the post-trade price and percentage impact of buying `<USD>` worth of token A from the pool, assuming flat pool-level liquidity (no tick-array walk). Orca pools only.

```bash
lp-inspect --rpc-url "$SOLANA_RPC_URL" impact Hp7... --size 5000
```

Example output:

```
Pool:          Hp7...
Current price: $24.187300
Trade size:    $5000
Price impact:  +1.2143%
Price after:   $24.481094
```

## What's implemented today

- Orca Whirlpool position parsing + full P&L / IL / Greeks breakdown (`position --protocol orca`)
- Raydium CLMM position parsing — minimal summary only (`position --protocol raydium`)
- Live pool watch via WebSocket `accountSubscribe` (`watch`, Orca)
- Depth estimate at ±1/2/5% using pool-level liquidity (`depth`, Orca)
- Constant-liquidity price-impact estimate for a USD trade size (`impact`, Orca)

This corresponds to the milestones in the original design doc: [`docs/superpowers/plans/2026-04-06-lp-inspector.md`](docs/superpowers/plans/2026-04-06-lp-inspector.md).

Known limitations: depth/impact use pool-level liquidity only — no tick-array walk yet, so results degrade for trades that cross tick boundaries. Token decimals are hardcoded (9/6) in the Orca position view.

## Local database (PostgreSQL + TimescaleDB)

The storage layer (positions, pool ticks, P&L history, rebalance events) lives in PostgreSQL with the TimescaleDB extension. SQL migrations are in [`migrations/`](migrations/) and are managed with [`sqlx-cli`](https://crates.io/crates/sqlx-cli).

### Spin up a local instance

```bash
docker run -d --name tick-liq-db \
  -e POSTGRES_PASSWORD=tickliq \
  -e POSTGRES_DB=tickliq \
  -p 5432:5432 \
  timescale/timescaledb:latest-pg16

export DATABASE_URL=postgres://postgres:tickliq@localhost:5432/tickliq
```

### Apply migrations

```bash
cargo install sqlx-cli --no-default-features --features native-tls,postgres
sqlx migrate run               # apply all up migrations
sqlx migrate revert            # roll back the most recent migration
```

Migrations are timestamp-prefixed and ship as paired `*.up.sql` / `*.down.sql` files. The `pool_ticks` and `pnl_history` tables are TimescaleDB hypertables partitioned on `ts`.

## Roadmap

- [`CLAUDE.md`](CLAUDE.md) — long-term vision: full LP manager with rebalancing engine, Drift perp hedging, and TimescaleDB-backed P&L history.
- [`docs/superpowers/plans/2026-04-07-followup-tasks.md`](docs/superpowers/plans/2026-04-07-followup-tasks.md) — concrete near-term follow-ups (correctness, accuracy, dev-ex, docs).

## License

License TBD — no `LICENSE` file is currently present in the repo.
