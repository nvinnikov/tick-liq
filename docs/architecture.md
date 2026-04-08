# Architecture

`tick-liq` is a layered Rust crate targeting an automated LP manager for Solana CLMM pools (Orca Whirlpools, Raydium CLMM). The design goal is that every layer above `data/` is pure — no I/O, no async, no global state — so the strategy logic is fully testable, deterministic, and reusable across the live runtime and the backtest replayer.

```
+-----------------+
|   lp-inspect    |   src/main.rs          read-only CLI
+--------+--------+
         |
         v
+-----------------+
|    strategy     |   src/strategy/        pure state machines
+-----------------+
         |
         v
+-----------------+
|      math       |   src/math/            pure CLMM primitives
+-----------------+

         +-----------------+         +-----------------+
         |      data       |         |    execution    |
         +-----------------+         +-----------------+
         |      Solana     |         |   Txs, hedging  |
         +-----------------+         +-----------------+

         +-----------------+
         |     storage     |   src/storage/  PostgreSQL + TimescaleDB
         +-----------------+
```

The layers depend on each other top-down. `math/` has no repo-internal dependencies. `strategy/` depends only on `math/`. `data/`, `execution/`, and `storage/` each sit as siblings of `strategy/` and deliberately communicate with it through small, data-only structs rather than via traits — the strategy layer never holds an RPC handle.

## Layer-by-layer

### `src/math/` — pure CLMM primitives

Pure functions over `u128` / `f64` implementing the Uniswap-V3-style sqrt-price math. No Solana deps, no `unwrap()` on user input. Invariants are enforced by property tests in `tests/*_props.rs`.

| Module          | Responsibility                                                                                     |
| --------------- | -------------------------------------------------------------------------------------------------- |
| `tick.rs`       | `tick ↔ sqrt_price ↔ price` conversions, tick spacing alignment.                                   |
| `liquidity.rs`  | `amounts_from_liquidity` / `liquidity_from_amounts` for all three price regimes (below, in, above). |
| `il.rs`         | Impermanent-loss fraction against a HODL baseline: `IL = (V_lp - V_hodl) / V_hodl`.                |
| `greeks.rs`     | Inventory Greeks: `δ = ∂x/∂P`, `γ = ∂²x/∂P²`. These are *inventory* deltas (units of token x per unit price move), not value deltas — see the module docs for the sign convention. |

**Key invariants** (enforced by proptests): amounts are non-negative, IL is non-positive, Greeks are zero outside the range, and `liquidity_from_amounts ∘ amounts_from_liquidity` is approximately the identity.

### `src/strategy/` — pure state machines

Each module is either a pure function or a state machine with explicit inputs and outputs. They share no global state, they are not async, and they do not hold handles to RPC or DB. Tests hit them directly with synthetic data.

| Module          | Responsibility                                                                                   |
| --------------- | ------------------------------------------------------------------------------------------------ |
| `fees.rs`       | `FeeTracker`: accumulates Q128 fee-growth deltas from successive pool snapshots into a running `(base, quote)` total. |
| `pnl.rs`        | `compute_pnl(PnlInput) -> PnlSnapshot`: takes entry composition `(entry_x, entry_y)`, current price, range, and fees-earned-so-far, and returns `fees_earned`, `il_quote`, `net`. |
| `range.rs`      | `RangeStrategy` trait with three implementations (`FixedWidth`, `VolatilityScaled`, `AsymmetricSkewed`). Returns `RangeRecommendation { lower_tick, upper_tick, expected_capital_efficiency_ppm }`. |
| `signal.rs`     | `SignalEngine`: pure state machine. Takes a `MarketTick`, returns `Hold` or `Rebalance { reason, target_range }`. Triggers in priority order: `OutOfRange`, `PnlBelowThreshold`, `FeesBelowFloor`, `Manual`. |
| `backtest.rs`   | `run_backtest`: replays a `Vec<BacktestTick>` through a `SignalEngine` + `RangeStrategy`, producing a `BacktestReport { total_fees, total_il_quote, net_pnl, num_rebalances, max_drawdown }`. Also ships a minimal inline CSV loader. |

**Key design notes:**

- **IL is present-value-anchored.** `compute_pnl` computes `V_hodl_now = entry_x * P_now + entry_y` and `il_quote = il_fraction * V_hodl_now`. The proptest `il_quote_equals_vlp_minus_vhodl` (in `tests/pnl_props.rs`) anchors the identity `il_quote == V_lp_now - V_hodl_now` across the full input space.
- **Capital efficiency follows the Uniswap V3 whitepaper §6.2.1.** `CE = 1 / (1 - (Pa/Pb)^(1/4))` — note the fourth root. For a ±10% range around the current price this gives `CE ≈ 20.4`.
- **`SignalEngine` is designed to be embeddable in both live runtime and backtests.** The same engine powers `run_backtest`; the backtest harness calls `engine.on_rebalance_executed(ts)` on each realized rebalance to keep the fee-window timer in sync. This is the same API the execution layer will call in production.
- **Realized vs. unrealized IL.** The backtest accumulates *realized* IL into `total_il_quote` only on position close (rebalance or terminal), while the drawdown path uses `total_fees + total_il_quote + snap.il_quote` so the live running P&L includes the current position's unrealized drift. This matches how a real portfolio would be marked.

### `src/data/` — I/O seams

Thin adapters over Solana RPC and price feeds. Each is constructed once and handed to the execution layer; the strategy layer never sees them.

| Module          | Responsibility                                                                            |
| --------------- | ----------------------------------------------------------------------------------------- |
| `rpc.rs`        | Pooled Solana HTTP RPC client.                                                            |
| `ws.rs`         | `accountSubscribe` WebSocket pool-state stream with automatic reconnect and backoff.      |
| `prices.rs`     | Pyth on-chain price feed + CEX (Binance/OKX) fallback. Verifies the Pyth program owner at construction. |

### `src/execution/` — transactions and hedging

The layer that actually moves tokens. Under active integration — the rebalance state machine and tx submitter have landed; the Drift perp hedge is WIP.

| Module            | Responsibility                                                                        |
| ----------------- | ------------------------------------------------------------------------------------- |
| `rebalance.rs`    | `RebalanceEngine`: consumes a `RebalanceSignal` from `strategy::signal`, walks the `close → collect fees → reopen` state machine, emits transaction requests. |
| `tx.rs`           | `TxSubmitter` trait: signs, submits, and confirms transactions with retry/backoff.    |
| `hedge.rs`        | Drift Protocol perp delta-hedge via Anchor CPI. **WIP** — currently a stub.           |

The end-to-end pipeline (`monitor → signal → rebalance build`) is exercised by `tests/e2e_pipeline.rs`.

### `src/storage/` — persistence

PostgreSQL + TimescaleDB. All time-series tables are hypertables partitioned on `ts`. Migrations live in `migrations/` as paired `*.up.sql` / `*.down.sql` files.

| Module          | Table(s)                | Purpose                                                                |
| --------------- | ----------------------- | ---------------------------------------------------------------------- |
| `positions.rs`  | `positions`             | CRUD for LP position records.                                          |
| `ticks.rs`      | `pool_ticks`            | Append-only pool-state snapshots (hypertable).                         |
| `pnl.rs`        | `pnl_history`           | Per-tick P&L snapshots (hypertable).                                   |
| `events.rs`     | `rebalance_events`      | Audit log of rebalance lifecycle events.                               |

Integration tests in `tests/storage_db.rs` run against a real Postgres instance when `TICKLIQ_DATABASE_URL` is set; they are skipped otherwise.

## The `lp-inspect` binary

`src/main.rs` is a thin clap-derive shell on top of the library. It shares its `analytics/`, `protocols/`, `display/`, and `rpc` submodules (under `src/`) with the older inspector code; the library crate under `src/lib.rs` is the path forward. Over time the inspector-side helpers will migrate into the layered modules.

The CLI subcommands listed in the README map onto these layers:

- `pool info` / `position` / `watch` / `depth` / `impact` → `analytics/` + `protocols/` (inspector legacy).
- `monitor` → polled variant of `watch` using `analytics::pnl::compute_il`.
- `backtest` → structured stub today; the real engine lives in `src/strategy/backtest.rs` and can be driven directly from Rust. A small integration example:

  ```rust
  use tick_liq::strategy::{backtest::*, range::FixedWidth, signal::SignalConfig};

  let ticks = load_ticks_csv("tests/fixtures/backtest_sample.csv")?;
  let cfg = BacktestConfig { /* ... */ };
  let report = run_backtest(&ticks, FixedWidth { half_width_frac: 0.05 }, cfg)?;
  println!("net P&L = {}", report.net_pnl);
  ```

## Testing strategy

- **Unit tests** live alongside each module.
- **Property tests** (`tests/*_props.rs`) use `proptest` to enforce layer invariants across wide input spaces — especially in `math/` and `strategy/pnl`. Invariants are the contract; prefer fixing the code over weakening the invariant.
- **Storage integration tests** (`tests/storage_db.rs`) are gated on `TICKLIQ_DATABASE_URL`.
- **E2E pipeline test** (`tests/e2e_pipeline.rs`) drives the `monitor → signal → rebalance build` path end-to-end against in-memory fakes of the `data/` and `execution/` seams.

## Error handling

- Use `anyhow::Result` for all fallible paths.
- No `unwrap()` or `expect()` in production code paths (tests are fine).
- Keypairs only via environment variables — never in config files or source.
- Always verify Solana account program owner before deserializing (see `data/prices.rs` for the Pyth example).
