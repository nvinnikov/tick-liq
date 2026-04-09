# External Integrations
_Last updated: 2026-04-09_

## Summary

The tool integrates with Solana (RPC + WebSocket), two on-chain CLMM protocols (Orca Whirlpools and Raydium CLMM) read via raw borsh deserialization, and PostgreSQL/TimescaleDB for persistence. Drift Protocol hedging is stubbed (size calculation only, no CPI wired). No HTTP APIs or price feed services are currently integrated.

## Solana RPC (JSON-RPC)

- **Purpose:** Fetch on-chain account data for pools, positions, tick arrays, SPL mint metadata
- **Crate:** `solana-client` 1.18 — `RpcClient` (synchronous, blocking calls wrapped in tokio `spawn_blocking` is NOT done — calls are blocking in current code)
- **Client location:** `src/rpc.rs` — `SolanaRpc` struct
- **Endpoint config:** `SOLANA_RPC_URL` env var (default: `https://api.devnet.solana.com`)
- **Key operations:**
  - `get_account` — raw account bytes for pool, position, tick array, SPL mint, Metaplex metadata PDAs
  - Owner verification via `verify_owner()` before every deserialization
- **No connection pooling currently** — a new `RpcClient` is created per command invocation

## Solana WebSocket (accountSubscribe)

- **Purpose:** Real-time pool account change notifications for the `watch` subcommand
- **Crate:** `tokio-tungstenite` 0.21
- **Location:** `src/data/ws.rs` — `watch_account()` function
- **Protocol:** Solana PubSub JSON-RPC 2.0 — `accountSubscribe` method, `base64` encoding, `confirmed` commitment
- **Features:** Exponential-backoff reconnect (1s base, 30s max), periodic ping/pong keepalive (30s interval, 10s pong timeout), graceful Ctrl+C shutdown via `tokio::sync::broadcast`
- **WS URL derivation:** `rpc_url.replace("https://", "wss://")` — no separate WS URL config

## Orca Whirlpools (on-chain, read-only)

- **Program ID:** `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc`
- **Location:** `src/protocols/orca.rs`
- **Access method:** Manual `borsh` deserialization of raw account bytes — does NOT use `anchor-client` or the Orca SDK at runtime (only `orca_whirlpools_core` for math utilities)
- **Account types deserialized:**
  - `WhirlpoolPool` — pool state (sqrt_price, tick, liquidity, fee growth)
  - `WhirlpoolPosition` — position state (liquidity, tick bounds, fee checkpoints, fees owed)
  - `TickArray` — 88-tick arrays for depth map rendering
- **PDAs used:** `["position", mint]` for positions; `["tick_array", whirlpool, start_tick_str]` for tick arrays
- **Write operations:** None currently (rebalance is dry-run only, no instructions built)

## Raydium CLMM (on-chain, read-only)

- **Program ID:** `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK`
- **Location:** `src/protocols/raydium.rs`
- **Access method:** Manual `borsh` deserialization — same pattern as Orca
- **Account types deserialized:**
  - `RaydiumPool` — pool state (sqrt_price_x64, tick_current)
  - `RaydiumPosition` — position state (tick bounds, liquidity)
- **Limitations:** Fee accrual and full P&L not yet computed for Raydium; output is minimal (pool address, price, ticks)

## Metaplex Token Metadata (on-chain, read-only)

- **Program ID:** `metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s`
- **Location:** `src/rpc.rs` — `fetch_token_symbol()` / `try_fetch_token_symbol()`
- **Purpose:** Resolve human-readable token symbols for display
- **Access method:** Manual byte layout parsing of metadata v1 account (65-byte header + name + symbol)
- **Fallback:** If metadata unavailable, uses first 8 chars of mint address

## PostgreSQL / TimescaleDB

- **Purpose:** Persist positions, tick-level liquidity snapshots, P&L history
- **Crates:** `sqlx-core` 0.8 + `sqlx-postgres` 0.8
- **Location:** `src/storage/mod.rs`, `src/storage/positions.rs`, `src/storage/schema.sql`
- **Connection:** `DATABASE_URL` env var; pool size 5 (`PgPoolOptions::new().max_connections(5)`)
- **Schema:**
  - `positions` — opened/closed LP positions with mint, protocol, tick range, entry price
  - `pool_ticks` — time-series tick liquidity snapshots (TimescaleDB hypertable when enabled)
  - `pnl_history` — per-position P&L snapshots over time (TimescaleDB hypertable when enabled)
- **Note:** `create_hypertable` calls are commented out in `schema.sql` — TimescaleDB hypertables must be activated manually

## Drift Protocol (planned, not wired)

- **Purpose:** Delta-neutral hedging via perp positions to offset negative LP delta
- **Location:** `src/execution/hedge.rs`
- **Current state:** `compute_hedge_size()` calculates required notional only; no Anchor CPI instructions are built or sent
- **Stub output:** Prints hedge plan with size/side; displays "Drift CPI not yet wired" note
- **Planned:** Anchor CPI calls to Drift v2 program

## Price Feeds

- **Current state:** No price feed integration present. All prices are derived directly from on-chain `sqrt_price` fields of pool accounts.
- **Planned (per CLAUDE.md):** Pyth Network oracle (primary) + CEX WebSocket (secondary) via `reqwest` / `tokio-tungstenite`

## Security Notes

- Keypairs: `LP_INSPECTOR_KEYPAIR` env var (base-58) — never in config files or code
- RPC endpoints may contain API keys — treat `SOLANA_RPC_URL` as a secret
- Program owner verification is enforced on every account fetch before deserialization (`src/rpc.rs:verify_owner`)

---

*Integration audit: 2026-04-09*
