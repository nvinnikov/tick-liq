# tick-liq

Automated LP (Liquidity Provider) manager for concentrated-liquidity (CLMM) pools on Solana. Inspects Orca Whirlpool and Raydium CLMM positions, computes real-time P&L / Greeks / IL, watches pools over WebSocket, evaluates rebalance signals, and generates dry-run execution plans.

## Architecture

```
src/
├── math/        Pure Rust CLMM math — zero external deps (IL, greeks, amounts, impact, sqrt_price)
├── protocols/   Borsh deserialization for Orca Whirlpool + Raydium CLMM, TickArray PDAs
├── analytics/   Thin orchestration over math/ + protocols/
├── data/        WebSocket pool subscription with reconnect + ping/pong
├── strategy/    Rebalance signal generator (pure logic, no Solana deps)
├── execution/   Dry-run rebalance planner + Drift hedge size estimator
├── storage/     sqlx + TimescaleDB schema scaffold (positions, pool_ticks, pnl_history)
├── display/     Formatted CLI output + ASCII liquidity histogram
└── rpc.rs       Solana RPC client with owner verification
```

## Prerequisites

- **Rust toolchain** via [rustup](https://rustup.rs/) (stable, edition 2021)
- **Solana RPC URL** — public endpoint works for read-only commands; private RPC (Helius, Triton, QuickNode) recommended for `watch` (public endpoints aggressively rate-limit WebSocket)

## Installation

```bash
git clone https://github.com/nvinnikov/tick-liq.git
cd tick-liq
cargo build --release
# Binary: target/release/lp-inspect
```

## Configuration

| Variable               | Default                         | Purpose |
| ---------------------- | ------------------------------- | ------- |
| `SOLANA_RPC_URL`       | `https://api.devnet.solana.com` | HTTP RPC. `watch` derives the WS URL automatically (`https://` → `wss://`). |
| `DATABASE_URL`         | —                               | Postgres connection string for `db migrate`. |
| `LP_INSPECTOR_KEYPAIR` | —                               | Base58 private key. Required by `rebalance` and `hedge` (env-only, never a file). |

```bash
export SOLANA_RPC_URL=https://mainnet.helius-rpc.com/?api-key=<KEY>
```

## Commands

```
lp-inspect [--rpc-url <URL>] [--db-url <URL>] <COMMAND>
```

### `position` — full P&L breakdown

```
lp-inspect position <MINT> [--protocol orca|raydium] [--entry-price <PRICE>]
```

Fetches position + pool on-chain. Prints: pool metadata, current vs. range price, in-range status, real token amounts (decimals from chain), accrued fees (USD), impermanent loss, net P&L, Greeks (delta, gamma). Pass `--entry-price` for accurate IL; omit to see fees only.

Raydium support is partial — pool address, price, tick, liquidity only.

```bash
lp-inspect position 4xj... --entry-price 150.50
lp-inspect position 4xj... --protocol raydium
```

### `watch` — live pool subscription (Orca)

```
lp-inspect watch <MINT>
```

Opens `accountSubscribe` WebSocket to the position's pool. Re-prints price / tick / in-range on every update. Exponential-backoff reconnect + ping/pong. Ctrl+C to stop.

### `depth` — tick-array liquidity map (Orca)

```
lp-inspect depth <POOL_ADDRESS>
```

Fetches 5 surrounding TickArray accounts, builds a bucketed liquidity distribution, renders an ASCII histogram, and estimates USD cost to move price ±1/2/5%.

### `impact` — price impact for a trade (Orca)

```
lp-inspect impact <POOL_ADDRESS> --size <USD>
```

Estimates post-trade price and % impact for a buy of `<USD>` worth of token A (constant-liquidity approximation).

### `strategy check` — rebalance signal

```
lp-inspect strategy check <MINT> [--near-edge-ticks 10] [--min-pnl 0.0] [--entry-price <PRICE>]
```

Fetches position + pool, computes net P&L, evaluates `should_rebalance()` (out-of-range / near-edge / P&L threshold). Prints `HOLD` or `REBALANCE` with reason.

### `rebalance` — dry-run execution plan

```
lp-inspect rebalance <MINT> --dry-run
```

Builds a close→collect→open instruction sequence and prints the plan. No transaction sent. Requires `LP_INSPECTOR_KEYPAIR` env var.

### `hedge` — Drift perp hedge estimate

```
lp-inspect hedge <MINT> --dry-run
```

Fetches position Greeks, computes required Drift perp size to neutralize delta (long if delta<0, short if delta>0). Prints plan. No CPI sent. Requires `LP_INSPECTOR_KEYPAIR` env var.

### `db migrate`

```
lp-inspect db migrate
```

Connects to Postgres (via `--db-url` or `DATABASE_URL`) and runs the schema migrations for `positions`, `pool_ticks`, and `pnl_history` tables.

## Example output

### `position` (Orca)

```
Pool:        Hp7...   fee 5 bps
Price:       $24.1873   range [$22.50, $26.00]   IN-RANGE (62%)
Amounts:     1.234 SOL   |   29.87 USDC
Fees:        $4.21    IL: -$0.83    Net: $3.38
Greeks:      delta=-0.0123  gamma=0.000041
```

### `strategy check`

```
Position:     4xj...
Tick current: -32184
Range:        [-33000, -31000]
Net P&L:      $3.38
Decision:     HOLD (position healthy)
```

### `hedge`

```
Hedge Plan (DRY RUN — no instruction sent)
Position:    4xj...
Delta:       -0.0123
Perp size:   $142.50
Side:        long  (offsetting negative delta)
Note:        Drift CPI not yet wired — size estimate only
```

## Development

```bash
cargo test                        # all unit + integration tests
cargo test --test math_golden     # golden vectors vs Orca SDK reference
cargo test --test math_props      # proptest property-based suite (8 invariants)
cargo clippy -- -D warnings       # lint
cargo fmt                         # format
```

## Test coverage

- 25+ unit tests across math, analytics, strategy, execution layers
- 8 proptest invariants (amounts ≥ 0, IL ≤ 0, delta sign, impact monotonicity, …)
- 20 golden reference vectors validated against Orca Whirlpool formulas

## TODO / Refactoring backlog

### High priority
- [ ] **Raydium analytics parity** — wire `analytics::*` + `display::table::print_position` into Raydium branch (currently prints raw fields only)
- [ ] **WS backoff reset** — `src/data/ws.rs` backoff doesn't reset after successful reconnect; saturates at 30s on flapping connections
- [ ] **`impact` tick-array walk** — current constant-liquidity approximation degrades for trades crossing tick boundaries; wire `build_distribution` into impact math

### Medium priority
- [ ] **Storage writes** — `PositionsRepo` is a stub; implement `insert_position`, `record_pnl` using `sqlx::query`
- [ ] **Rebalance execution** — `src/execution/rebalance.rs` builds the plan but doesn't construct actual Solana instructions (`close_position` / `collect_fees` / `open_position` CPI calls)
- [ ] **Drift CPI** — `src/execution/hedge.rs` estimates size but doesn't build the Drift instruction; wire `anchor-client` CPI
- [ ] **Entry-price persistence** — currently `--entry-price` is ephemeral; persist to `$XDG_CACHE_HOME/lp-inspect/<mint>` so IL is accurate across `watch` sessions

### Low priority
- [ ] **`src/math/` amounts module** — `compute_token_amounts` still lives in `analytics/amounts.rs` (uses `orca_whirlpools_core`); move pure math to `src/math/amounts.rs` once deps are decoupled
- [ ] **RPC timeout + retry** — `src/rpc.rs` has no timeout or retry; add configurable `--rpc-timeout` and exponential backoff
- [ ] **Raydium field-order verification** — `src/protocols/raydium.rs` field order not validated against program source; add fixture-based parse test
- [ ] **LICENSE file** — license TBD

## Roadmap

- [`CLAUDE.md`](CLAUDE.md) — full vision: automated rebalancing, Drift perp hedging, TimescaleDB P&L history
- [`docs/superpowers/plans/2026-04-07-followup-tasks.md`](docs/superpowers/plans/2026-04-07-followup-tasks.md) — completed F1–F14 task breakdown
