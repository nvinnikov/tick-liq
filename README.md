# tick-liq

`lp-inspect` is a read-only CLI for inspecting concentrated-liquidity (CLMM) positions on Solana. It parses Orca Whirlpool positions (with partial Raydium CLMM support), prints a real-time P&L / Greeks breakdown, watches a pool over WebSocket, and answers depth and price-impact questions against on-chain pool state.

This binary is the first slice of a larger automated LP manager — see [`CLAUDE.md`](CLAUDE.md) for the broader vision (rebalancing engine, perp hedging, TimescaleDB-backed history).

## Prerequisites

- **Rust toolchain** via [rustup](https://rustup.rs/) (stable, edition 2021)
- **A Solana RPC URL.** A public endpoint works for read-only inspection; a private RPC (Helius, Triton, QuickNode) is recommended for `watch` — public endpoints rate-limit WebSocket subscriptions aggressively.

## Installation

```bash
git clone https://github.com/nvinnikov/tick-liq.git
cd tick-liq
cargo build --release
```

The binary is produced at `target/release/lp-inspect`.

## Configuration

| Variable         | Default                         | Purpose                                                                                             |
| ---------------- | ------------------------------- | --------------------------------------------------------------------------------------------------- |
| `SOLANA_RPC_URL` | `https://api.devnet.solana.com` | HTTP RPC endpoint. `watch` derives the WebSocket URL by swapping `https://` → `wss://` automatically. |

You can also pass `--rpc-url <URL>` on the command line to override the environment variable for any single invocation.

```bash
export SOLANA_RPC_URL=https://mainnet.helius-rpc.com/?api-key=<YOUR_KEY>
```

## Commands

All subcommands share the global `--rpc-url` flag:

```
lp-inspect [--rpc-url <URL>] <COMMAND>
```

### `position` — full P&L breakdown

```
lp-inspect position <MINT> [--protocol orca|raydium] [--entry-price <PRICE>]
```

| Argument        | Default | Description                                                |
| --------------- | ------- | ---------------------------------------------------------- |
| `<MINT>`        | —       | Position NFT mint address                                  |
| `--protocol`    | `orca`  | Protocol: `orca` or `raydium`                              |
| `--entry-price` | —       | Entry price in quote/base (USD). Required for accurate IL. |

Fetches the position and its pool on-chain, then prints: pool metadata, current vs. range price, in-range status, token amounts, accrued fees (USD), impermanent loss, net P&L, and Greeks (delta, gamma). Pass `--entry-price` to compute IL relative to your actual open price; without it IL is reported as 0.

Raydium support is partial — prints pool address, price, tick, and liquidity only.

```bash
./target/release/lp-inspect --rpc-url "$SOLANA_RPC_URL" position 4xj... --protocol orca
./target/release/lp-inspect --rpc-url "$SOLANA_RPC_URL" position 4xj... --protocol raydium
```

### `watch` — live pool subscription (Orca only)

```
lp-inspect watch <MINT>
```

Resolves the position's pool, opens a WebSocket `accountSubscribe` to that pool, and re-prints price / tick / in-range status on every on-chain update. Press Ctrl+C to stop.

```bash
./target/release/lp-inspect --rpc-url "$SOLANA_RPC_URL" watch 4xj...
```

### `depth` — liquidity around the current price (Orca only)

```
lp-inspect depth <POOL_ADDRESS>
```

Reads pool-level liquidity and estimates the USD trade size required to move the price ±1%, ±2%, ±5%.

```bash
./target/release/lp-inspect --rpc-url "$SOLANA_RPC_URL" depth Hp7...
```

### `impact` — price impact for a specific trade (Orca only)

```
lp-inspect impact <POOL_ADDRESS> --size <USD>
```

Estimates the post-trade price and percentage impact of buying `<USD>` worth of token A from the pool, assuming constant pool-level liquidity.

```bash
./target/release/lp-inspect --rpc-url "$SOLANA_RPC_URL" impact Hp7... --size 50000
```

## Example output

### `position` (Orca)

```
Pool:        Hp7...   fee 5 bps
Price:       $24.1873   range [$22.50, $26.00]   IN-RANGE (62%)
Amounts:     1.234 A   |   29.87 B
Fees:        $4.21    IL: -$0.83    Net: $3.38
Greeks:      delta=-0.0123  gamma=0.000041
```

### `watch`

```
[14:02:11 UTC] Pool update received

Pool:      Hp7...
Price:     $24.1901
Tick:      -32184
In range:  YES
Liquidity: 1284732001
```

### `depth`

```
Liquidity Distribution  (pool liquidity: 1284M)
──────────────────────────────────────────────────
  +1%  (~$24.4292): $4123 needed to buy  |  $4087 needed to sell
  +2%  (~$24.6710): $8210 needed to buy  |  $8113 needed to sell
  +5%  (~$25.3967): $20114 needed to buy  | $19782 needed to sell

Note: uses pool-level liquidity. Tick-array depth map coming in a future update.
```

### `impact`

```
Pool:          Hp7...
Current price: $24.187300
Trade size:    $50000
Price impact:  +1.2143%
Price after:   $24.481094
```

## Development

```bash
# Run all tests
cargo test

# Property-based math tests
cargo test --test math_tests

# Lint (warnings treated as errors)
cargo clippy -- -D warnings

# Format
cargo fmt
```

## What's implemented

- Orca Whirlpool position parsing + full P&L / IL / Greeks breakdown (`position --protocol orca`)
- Raydium CLMM position parsing — minimal summary only (`position --protocol raydium`)
- Live pool watch via WebSocket `accountSubscribe` (`watch`, Orca)
- Depth estimate at ±1/2/5% using pool-level liquidity (`depth`, Orca)
- Constant-liquidity price-impact estimate for a USD trade size (`impact`, Orca)

Known limitations: `depth` and `impact` use pool-level liquidity only — no tick-array walk, so results degrade for trades that cross tick boundaries. Token decimals are hardcoded (9/6) in the Orca position view.

## Roadmap

- [`CLAUDE.md`](CLAUDE.md) — long-term vision: full LP manager with rebalancing engine, Drift perp hedging, and TimescaleDB-backed P&L history.
- [`docs/superpowers/plans/2026-04-07-followup-tasks.md`](docs/superpowers/plans/2026-04-07-followup-tasks.md) — concrete near-term follow-ups (correctness, accuracy, dev-ex, docs).

## License

License TBD — no `LICENSE` file is currently present in the repo.
