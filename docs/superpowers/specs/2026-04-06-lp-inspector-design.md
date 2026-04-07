# Design: LP Inspector CLI

**Date:** 2026-04-06
**Status:** Approved
**Horizon:** 8 weeks to working prototype

---

## Context

tick-liq is an automated LP manager for CLMM pools on Solana. The project is in pre-implementation phase. Primary goal: learn Rust + Solana through a real project, with a path to open-source tooling useful to other developers, traders, and market makers.

**Key constraint:** Do not implement CLMM math from scratch. Use validated protocol libraries (`orca_whirlpools_core`, `raydium-clmm`) to avoid precision bugs in financial calculations.

---

## Product

A developer-friendly CLI tool for inspecting CLMM positions on Solana. Like `foundry cast` for Ethereum developers — powerful, terminal-native, no UI required.

Target users: LP managers, market makers, Solana developers integrating with Orca/Raydium.

### Commands

```bash
# Full position breakdown: P&L, IL, fees, Greeks
lp-inspect position <MINT>

# Real-time monitoring (WebSocket)
lp-inspect watch <MINT>

# Liquidity distribution around current price
lp-inspect depth <POOL>

# Price impact for a specific trade size
lp-inspect impact <POOL> --size 50000
```

### Example output — `position`

```
Position: 7xK...abc  (Orca SOL/USDC 0.05%)
──────────────────────────────────────────
Range:      $142.50 — $158.30
Current:    $151.20  ✓ IN RANGE (63%)

Amounts:    12.34 SOL  +  456.78 USDC

P&L:
  Fees:    +$23.45  (+1.25%)
  IL:       -$8.90  (-0.48%)
  Net:     +$14.55  (+0.77%)

Delta:  -0.34   Gamma: 0.02
```

### Example output — `depth`

```
Liquidity Distribution (±10% from $151.20)
───────────────────────────────────────────
$145  ████████░░░░░░░  2.3M
$148  ██████████████░  4.1M
$151  ███████████████  5.2M  ← current
$154  ████████████░░░  3.8M
$157  ██████░░░░░░░░░  1.9M

Price Impact:
  +1%  (~$153): $45,000 buy needed
  +2%  (~$154): $98,000 buy needed
  +5%  (~$159): $280,000 buy needed
  -1%  (~$150): $38,000 sell needed
```

---

## Architecture

```
tick-liq/
├── Cargo.toml
└── src/
    ├── main.rs              # CLI entry point (clap)
    ├── rpc.rs               # Solana RPC client, account fetching
    ├── protocols/
    │   ├── mod.rs
    │   ├── orca.rs          # Orca Whirlpool account deserialization
    │   └── raydium.rs       # Raydium CLMM account deserialization
    ├── analytics/
    │   ├── mod.rs
    │   ├── pnl.rs           # fees earned, IL, net P&L
    │   ├── greeks.rs        # delta, gamma
    │   └── depth.rs         # liquidity density, price impact
    └── display/
        ├── mod.rs
        └── table.rs         # formatted terminal output
```

### Module responsibilities

| Module | Knows about | Does NOT know about |
|--------|-------------|---------------------|
| `rpc.rs` | Solana RPC, account bytes | protocols, math |
| `protocols/` | on-chain account layouts, deserialization | analytics, display |
| `analytics/` | math (via protocol libs), position structs | Solana, display |
| `display/` | formatting, terminal output | Solana, math |

Each module is independently testable.

### Key dependencies

| Crate | Purpose |
|-------|---------|
| `orca_whirlpools_core` | Validated Orca CLMM math |
| `raydium-clmm` | Validated Raydium CLMM math |
| `solana-client` + `solana-sdk` | RPC and account handling |
| `clap` v4 (derive) | CLI commands |
| `anyhow` | Error handling |
| `tokio` (full) | Async runtime for RPC and WebSocket |

Math is NOT implemented from scratch — protocol libraries are the source of truth for all CLMM calculations.

---

## Weekly Plan

### Week 1: Foundation
- Workspace setup, `Cargo.toml` with dependencies
- `rpc.rs`: connect to devnet, fetch a single account
- `protocols/orca.rs`: deserialize Whirlpool pool account
- **Rust learned:** structs, `anyhow`, basic async/await

### Week 2: First working command
- `lp-inspect position <MINT>` — show amounts, range, in/out status
- `display/table.rs`: formatted terminal output
- **Rust learned:** `clap` derive macros, traits for formatting

### Week 3: P&L and analytics
- `analytics/pnl.rs`: fees earned, IL, net P&L using protocol math libs
- Greeks: delta, gamma
- **Rust learned:** working with external crates, numeric types

### Week 4: Liquidity depth + price impact
- `analytics/depth.rs`: read tick bitmap accounts, build distribution
- `lp-inspect depth <POOL>` and `lp-inspect impact`
- **Rust learned:** iterating on-chain data, swap algorithm for price impact

### Week 5-6: Raydium + polish
- `protocols/raydium.rs`: same functionality for Raydium CLMM
- `lp-inspect watch` with real WebSocket subscription
- README with examples

### Week 7-8: Buffer
- Catch-up on anything behind schedule
- Tests, documentation
- Share in Solana dev community (Discord, Twitter)

---

## What we are NOT building in this phase

- Automated rebalancing / transaction execution
- Delta hedge via Drift Protocol
- PostgreSQL / TimescaleDB storage
- Range optimizer
- Multi-position dashboards
- Production observability

These are natural next phases once the analytics foundation is solid.

---

## Open Source Strategy

Publish to GitHub after Week 6 when Orca + Raydium support is working. The tool demonstrates real value immediately — developers can run it against live positions. Promote in Solana dev communities where there is currently no equivalent tool.