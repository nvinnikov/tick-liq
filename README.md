# tick-liq

**A liquidity-provider manager for Solana CLMM pools, written in Rust.** Reads on-chain
Orca Whirlpool / Raydium positions, computes real-time P&L (fees − impermanent loss),
option-style Greeks (delta/gamma), price impact, and tick-level liquidity depth — then
generates rebalance and delta-hedge signals. Includes an offline CLMM backtester built
on the same math that powers the live inspector.

Built as a from-scratch exploration of concentrated-liquidity market microstructure:
the CLMM math has **zero external dependencies** and is validated against the Orca
Whirlpool SDK via golden vectors and property-based tests.

Stack: **Rust · Tokio (async) · Solana RPC/WebSocket · CLMM math · PostgreSQL/TimescaleDB**

```
Pool:        Hp7...   fee 5 bps
Price:       $24.1873   range [$22.50, $26.00]   IN-RANGE (62%)
Amounts:     1.234 SOL   |   29.87 USDC
Fees:        +$4.21    IL: -$0.83    Net: +$3.38
Greeks:      delta=-0.0123  gamma=0.000041
Decision:    HOLD (position healthy, net P&L positive)
```

> ⚠️ **Disclaimer — educational / research use only.** This is a personal research and
> portfolio project, **not financial advice** and not a turnkey trading system. Execution
> paths (`rebalance`, `hedge`) are **dry-run only** — no transactions or CPI are sent.
> The backtester uses simulated price paths. Nothing here is audited. Use at your own risk;
> DeFi positions can lose value. Never commit private keys — supply them via environment
> variables only (see [Configuration](#configuration)).

---

## Architecture

```
src/
├── math/        Pure Rust CLMM math — zero external deps (IL, greeks, amounts, impact, sqrt_price)
├── protocols/   Borsh deserialization for Orca Whirlpool + Raydium CLMM, TickArray PDAs
├── analytics/   Thin orchestration over math/ + protocols/
├── data/        WebSocket pool subscription with reconnect + ping/pong
├── strategy/    Rebalance signal generator (pure logic, no Solana deps)
├── execution/   Dry-run rebalance planner + Drift hedge size estimator
├── backtest/    GBM price simulator — runs full data→math→signal pipeline offline
├── storage/     PostgreSQL + TimescaleDB schema scaffold (positions, pool_ticks, pnl_history)
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

| Variable                    | Default                         | Purpose |
| --------------------------- | ------------------------------- | ------- |
| `SOLANA_RPC_URL`            | `https://api.devnet.solana.com` | HTTP RPC. `watch` derives the WS URL automatically (`https://` → `wss://`). |
| `DATABASE_URL`              | —                               | Postgres connection string for `db migrate`. |
| `LP_INSPECTOR_KEYPAIR`      | —                               | Base58 private key. Required by `rebalance` and `hedge` (env-only, never a file). |
| `POSTGRES_PASSWORD`         | —                               | DB password for `docker compose up` (no default; compose binds Postgres to `127.0.0.1` only). Must match the password in `DATABASE_URL`. |
| `TELEGRAM_BOT_TOKEN`        | —                               | Bot API token. Required by `watch --telegram` (env-only, never a file). |
| `TELEGRAM_CHAT_ID`          | —                               | Authorized chat ID. Commands from any other chat are ignored. Required by `watch --telegram`. |
| `TELEGRAM_ALLOWED_USER_IDS` | —                               | Comma-separated Telegram user IDs allowed to send commands (`/approve`, `/pause`, …). Chat membership alone is not enough. Required by `watch --telegram`. |
| `METRICS_LISTEN`            | —                               | `host:port` (e.g. `0.0.0.0:9100`) to serve Prometheus metrics over HTTP (pull/scrape). Wins over `METRICS_PUSH_URL` if both are set. |
| `METRICS_PUSH_URL`          | —                               | Push-gateway URL (VictoriaMetrics `/api/v1/import/prometheus`). Enables push mode when `METRICS_LISTEN` is unset. |
| `METRICS_PUSH_INTERVAL_SECS`| `15`                            | Push interval in seconds for `METRICS_PUSH_URL`. Falls back to 15 on a malformed value. |
| `COINBASE_SYMBOL`           | —                               | Coinbase product id (e.g. `SOL-USD`) for the observational secondary price feed in `watch` (also `--coinbase-symbol`). Metrics-only; does not affect P&L or rebalancing. |

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

### `backtest` — offline LP simulation

```
lp-inspect backtest \
  --entry-price 150.0 \
  --price-lower 130.0 \
  --price-upper 170.0 \
  [--fee-bps 5] [--capital 10000] [--days 30] \
  [--volatility 0.80] [--daily-volume 1000000] \
  [--tick-spacing 64] [--rebalance] [--seed 42]
```

Simulates a CLMM position over a Geometric Brownian Motion price path using the same `src/math/` functions that power the live inspector (IL, fee accrual). Shows per-day P&L table and summary stats. Add `--rebalance` to trigger automatic range resets when the position goes out of range.

```
lp-inspect backtest --entry-price 150 --price-lower 130 --price-upper 170 --days 30
```

```
Backtest — CLMM LP Simulation
────────────────────────────────────────────────────────────
Entry:         $150.0000   Range: $130.0000 – $170.0000
Fee:           5 bps   Vol: 80% ann.   Volume: $1000000/day
Capital:       $10000   Days: 30
────────────────────────────────────────────────────────────
 Day       Price   InRange     CumFees          IL      NetP&L
────────────────────────────────────────────────────────────
   1    157.5246       yes       50.00       -2.99       47.01
   4    156.4987       yes      200.00       -2.25      197.75
   7    134.5144       yes      350.00      -14.82      335.18
  10    138.2244       yes      450.00       -8.35      441.65
  13    125.9890        NO      500.00      -25.54      474.46
  17    123.8809        NO      550.00      -25.54      524.46
  20    118.4441        NO      550.00      -25.54      524.46
  23    121.1127        NO      550.00      -25.54      524.46
  26    116.0894        NO      550.00      -25.54      524.46
  30    116.6243        NO      550.00      -25.54      524.46
════════════════════════════════════════════════════════════
Fees earned:   $550.00  (5.5% of capital)
Imperm. loss:  $-25.54  (-0.3% of capital)
Net P&L:       $524.46  (5.2% of capital)
Fee APY:       66.9%
Days in range: 11/30 (37%)
────────────────────────────────────────────────────────────
Risk metrics (annualised, rf=0)
Volatility:    42.0%
Sharpe:        1.83
Sortino:       2.91
Max drawdown:  -4.1%
Calmar:        15.2
```

Risk metrics are computed from the per-day equity curve by `src/math/metrics.rs` (pure, zero-dep, golden-tested) and print for both the GBM and the DB-replay (`--pool`) backtests. Undefined metrics (e.g. a perfectly flat return series) render as `n/a`.

### `backfill` — seed real history from GeckoTerminal (no API key)

```
lp-inspect backfill --pool <ADDR> --from <YYYY-MM-DD> --to <YYYY-MM-DD> \
  [--timeframe day|hour|minute] [--fee-bps 4] --pool-liquidity <L> \
  [--decimals-a 9] [--decimals-b 6] [--tick-spacing 64] [--dry-run]
```

Pulls real price + USD-volume history for a Solana pool from the free, key-less
[GeckoTerminal API](https://www.geckoterminal.com/dex-api) and synthesises `pool_ticks`
rows so `backtest --pool` can replay **real data** instead of a synthetic GBM path.

Price → `sqrt_price`/`tick_current`; per-candle USD volume → a running
`fee_growth_global_b` accumulator (`Δfg = volume·fee_rate·10^decimals_b·2^64 / pool_liquidity`),
so a replayed position earns exactly its share `volume·fee_rate·(position_L / pool_L)`
of pool fees — verified by a golden round-trip test against `db_replay`.

`--dry-run` fetches, synthesises, and previews the window (no database needed). Without it,
rows are written to Postgres via `--db-url`/`DATABASE_URL` (idempotent on `(pool, slot)`).
Liquidity is approximated as a constant (`--pool-liquidity`, read from `depth` or on-chain);
fees are attributed to the quote side — accepted approximations for research.

```bash
# Preview a window with no DB:
lp-inspect backfill --pool Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE \
  --from 2026-06-01 --to 2026-06-15 --fee-bps 4 --pool-liquidity 1000000000000000 --dry-run
```

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
