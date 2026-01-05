# tick-liq: Automated LP Manager for Solana CLMM

## What This Is

Rust CLI that monitors Orca Whirlpool and Raydium CLMM positions in real time, calculates P&L (fees minus IL), and executes automated range rebalancing with optional delta hedging via Drift Protocol perps. Designed to deploy $20-30k in SOL/USDC and SOL/USDT 5bps pools with a mandatory 2-week shadow mode before any real capital moves.

## Core Value

**Profitable, hands-off LP with automated rebalancing and delta hedge — verifiable in shadow before any capital is at risk.**

## Requirements

### Validated

- ✓ CLI commands: position, watch, depth, impact, strategy, backtest — Phase 0
- ✓ CLMM math: tick↔price, liquidity/amounts, IL, Greeks — Phase 0
- ✓ Dry-run rebalance + Drift perp hedge simulation — Phase 0
- ✓ GBM backtest simulator — Phase 0
- ✓ TimescaleDB schema scaffolded (pool_ticks, pnl_history) — Phase 0
- ✓ 25+ unit + property-based tests — Phase 0

### Active

- [ ] TimescaleDB writes in watch mode (pool_ticks + pnl_history per tick)
- [ ] Shadow mode (--shadow flag — full logic, no signing)
- [ ] Real-data backtest from TimescaleDB (replaces GBM)
- [ ] Slippage guard before transaction submission
- [ ] Live execution via Anchor CPI (Orca + Drift perp)
- [ ] Risk limits with configurable per-limit actions
- [ ] Telegram bot (/approve, /status, /pause, /report)

### Out of Scope

- Jito bundle integration — deferred; slippage guard sufficient for v1 live
- Historical tick import (Birdeye/Flipside) — backtest uses live-collected ticks only
- Multi-pool simultaneous management — single pool focus for shadow validation
- Web dashboard — CLI + Telegram sufficient for operator UX

## Context

- **Stack**: Rust 1.78, Solana 1.18, Anchor 0.29, sqlx + TimescaleDB, tokio-tungstenite
- **Arch**: Three-layer (Data → Strategy → Execution) with pure-Rust CLMM math module
- **Capital**: $20-30k SOL/USDC 5bps + SOL/USDT 5bps pools on mainnet
- **Shadow gate**: Minimum 2 weeks runtime + zero rebalance errors + explicit `--live` flag
- **Tick schema**: One row in pool_ticks (full state snapshot) + one row in pnl_history (P&L delta) per WebSocket event

## Constraints

- **Security**: Keypairs only via env vars — never in config files or code
- **Safety**: No live transaction submission until shadow gate passed (2 weeks + zero errors + manual approval)
- **DB**: Non-blocking async writes — tick latency must not be impacted by storage I/O
- **Risk**: All limit thresholds configurable via CLI flags; per-limit actions: drawdown→close position, IL→pause rebalancing, margin-ratio→close Drift hedge only

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Skip Jito for now | Slippage guard sufficient to prevent MEV losses at this capital size | — Pending |
| Backtest on live ticks only | No historical import complexity; 2-week shadow window provides enough data | — Pending |
| Risk limits are per-type configurable | Different risk types warrant different responses (IL is recoverable, drawdown may not be) | — Pending |
| Shadow gate requires manual --live flag | Prevents accidental graduation to live even if automated criteria pass | — Pending |

---
*Last updated: 2026-04-09 after initial project initialization*
