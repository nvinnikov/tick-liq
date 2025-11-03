# ARCHITECTURE.md — System Architecture

## Pattern

Layered service architecture with clear separation between data ingestion, strategy computation, and on-chain execution. All layers are async (Tokio). Math layer is pure (no I/O).

## Layers

```
┌─────────────────────────────────────────────────────────────┐
│                        LP Manager                           │
├──────────────┬──────────────────┬──────────────────────────┤
│  Data Layer  │  Strategy Layer  │    Execution Layer        │
│              │                  │                            │
│ Solana RPC   │ IL Calculator    │ Rebalance Engine          │
│ WebSocket    │ Fee Tracker      │ Hedge Executor            │
│ Pool State   │ Range Optimizer  │ Transaction Builder       │
│ Price Feed   │ Signal Generator │ Anchor CPI calls          │
├──────────────┴──────────────────┴──────────────────────────┤
│                     Storage Layer                           │
│              PostgreSQL / TimescaleDB                       │
│         (positions, ticks, P&L history, events)            │
└─────────────────────────────────────────────────────────────┘
```

## Module Responsibilities

### `src/math/` — Pure computation, no I/O
- `clmm.rs` — tick ↔ price conversion, amounts from liquidity `(x, y) = f(L, P, Pa, Pb)`
- `il.rs` — impermanent loss for standard AMM and concentrated liquidity
- `greeks.rs` — position delta and gamma; delta = `-L / (2√P * P)` when price in range

### `src/data/` — External state ingestion
- `rpc.rs` — Solana RPC client (connection pool); fetches pool/position accounts
- `pool.rs` — Account structs for Orca Whirlpool and Raydium CLMM; borsh deserialization
- `price_feed.rs` — Pyth oracle + CEX WebSocket price streams

### `src/strategy/` — Decision logic (pure, no execution)
- `monitor.rs` — Polls/subscribes to position state; triggers strategy evaluation
- `rebalance.rs` — Signal: rebalance when price deviates N% from range center
- `range_optimizer.rs` — Given volatility/fee data, compute optimal `[Pa, Pb]`

### `src/execution/` — On-chain actions
- `rebalance_exec.rs` — Builds and submits: close position → collect fees → open new position
- `hedge.rs` — Opens/adjusts Drift Protocol short perp to neutralize delta

### `src/storage/` — Persistence
- `db.rs` — `sqlx::PgPool`; writes snapshots, tick events, P&L records

### `src/config.rs` — Config struct deserialized from TOML + env vars

### `src/main.rs` — CLI entry point (clap); subcommands route to layers

## Data Flow

1. **WebSocket** (pool account changes) → `data/pool.rs` → decoded `PoolState`
2. `PoolState` + `PositionState` → `math/` → `ILResult`, `GreeksResult`
3. Strategy layer evaluates signals → optional `RebalanceSignal`
4. Signal → `execution/rebalance_exec.rs` → Solana transaction
5. All state snapshots → `storage/db.rs` → PostgreSQL

## Entry Points

- CLI: `src/main.rs` — `lp-manager pool info`, `position monitor`, `backtest`
- Library: `src/lib.rs` — exposes math and strategy modules for testing

## Key Design Decisions

- Math is completely decoupled from I/O — enables pure unit testing and backtesting
- Backtester replays historical swaps through the same math layer as live monitoring
- Dry-run mode in execution layer simulates transactions without submitting
- Delta hedge is optional / layered on top — can disable without affecting core