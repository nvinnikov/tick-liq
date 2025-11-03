# INTEGRATIONS.md — External Integrations

## Solana RPC

- **Providers:** Helius (preferred) or QuickNode
- **Access pattern:** Connection pool — single client reused across requests, not recreated per call
- **Crate:** `solana-client`
- **Config key:** RPC URL via environment variable

## Solana WebSocket

- **Purpose:** Subscribe to pool account changes for real-time state updates
- **Crate:** `tokio-tungstenite` with reconnect logic
- **Target latency:** < 500ms from pool change to position update

## On-Chain Programs (Anchor CPI)

| Protocol | Program | Usage |
|----------|---------|-------|
| Orca Whirlpools | Whirlpool program | Read pool state, open/close positions |
| Raydium CLMM | CLMM program | Read pool state, open/close positions |
| Drift Protocol | Drift v2 | Open/close perp positions for delta hedge |

- Deserialization: use protocol-specific crates (`whirlpool` crate for Orca) or manual `borsh`
- Always verify `program_owner` on accounts before deserializing

## Price Feeds

- **Primary:** Pyth Network oracle (on-chain, low-latency)
- **Secondary:** CEX WebSocket (for redundancy / cross-validation)
- **Crate:** `reqwest` for HTTP, `tokio-tungstenite` for WebSocket streams

## PostgreSQL / TimescaleDB

- **Purpose:** Persist position snapshots, tick-level P&L, events
- **Crate:** `sqlx` 0.7 with `postgres`, `runtime-tokio`, `chrono` features
- **Extension:** TimescaleDB for time-series hypertables on tick/P&L data
- **Connection:** Pool via `sqlx::PgPool`

## Orca / Raydium Historical Data

- **Source:** Orca API or raw transaction parsing
- **Purpose:** Backtester — replay historical swaps against a position
- **Access:** `reqwest` HTTP client

## Security Notes

- Keypairs: environment variables only (`SOLANA_KEYPAIR_PATH` or base58 env var)
- Never log keypair material
- RPC endpoints may contain API keys — treat as secrets
