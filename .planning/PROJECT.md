# tick-liq: Automated LP Manager for Solana CLMM

## What This Is

Rust CLI that monitors Orca Whirlpool and Raydium CLMM positions in real time, calculates P&L (fees minus IL), and executes automated range rebalancing with optional delta hedging via Drift Protocol perps. Supports a mandatory 2-week shadow mode before any real capital moves, and wraps execution in a Telegram approval gate so the operator approves each rebalance before it fires.

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
- ✓ TimescaleDB writes in watch mode (pool_ticks + pnl_history per tick) — v1.0
- ✓ Shadow mode (`--shadow` flag — full logic, no signing) — v1.0
- ✓ Real-data backtest from TimescaleDB (replaces GBM) — v1.0
- ✓ Slippage guard before transaction submission — v1.0
- ✓ Live execution via Anchor CPI to Orca Whirlpool — v1.0
- ✓ WALLET_KEYPAIR env var gate at startup — v1.0
- ✓ Risk limits (drawdown/IL/margin-ratio) with per-limit actions — v1.0 ⚠ LP/hedge close CPIs deferred
- ✓ Telegram bot (/approve, /status, /pause, /resume, /report) — v1.0

### Active (v1.1 candidates)

- [ ] LIVE-02: Drift Protocol perp hedge update in the same rebalance cycle (deferred from v1.0)
- [ ] LIVE-04: LP↔Drift atomicity — rollback if hedge update fails (blocked by LIVE-02)
- [ ] RISK-01 (full): LP close CPI on drawdown breach (deferred — requires LIVE-02)
- [ ] RISK-03 (full): Drift hedge close CPI on margin breach (deferred — requires LIVE-02)
- [ ] E2E integration tests with funded devnet wallet

### Out of Scope

- Jito bundle integration — deferred; slippage guard sufficient for v1 live
- Historical tick import (Birdeye/Flipside) — backtest uses live-collected ticks only
- Multi-pool simultaneous management — single pool focus for shadow validation
- Web dashboard — CLI + Telegram sufficient for operator UX

## Context

- **Stack**: Rust 1.78, Solana 1.18, Anchor 0.29, sqlx + TimescaleDB, tokio-tungstenite, teloxide 0.13
- **Arch**: Three-layer (Data → Strategy → Execution) with pure-Rust CLMM math module + bot layer
- **LOC**: ~7,600 Rust (src/)
- **Capital**: $20-30k SOL/USDC 5bps + SOL/USDT 5bps pools on mainnet
- **Shadow gate**: Minimum 2 weeks runtime + zero rebalance errors + explicit `--live` flag
- **Approval gate**: Each rebalance requires `/approve` in Telegram within configurable timeout (default 300s)
- **Known tech debt**: LP close CPI (`OrcaExecutor::execute_close_position`) and Drift hedge close CPI not yet implemented — drawdown/margin-breach actions log deferred intent instead of executing

## Constraints

- **Security**: Keypairs only via env vars — never in config files or code
- **Safety**: No live transaction submission until shadow gate passed (2 weeks + zero errors + manual approval)
- **DB**: Non-blocking async writes — tick latency must not be impacted by storage I/O
- **Risk**: All limit thresholds configurable via CLI flags; per-limit actions: drawdown→close position, IL→pause rebalancing, margin-ratio→close Drift hedge only

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Skip Jito for v1 | Slippage guard sufficient to prevent MEV losses at $20-30k capital | ✓ Good — slippage guard shipped, no MEV issues at this size |
| Backtest on live ticks only | No historical import complexity; 2-week shadow window provides enough data | ✓ Good — real-data backtest functional with TimescaleDB |
| Risk limits are per-type configurable | Different risk types warrant different responses (IL is recoverable, drawdown may not be) | ✓ Good — three independent thresholds with distinct actions |
| Shadow gate requires manual --live flag | Prevents accidental graduation to live even if automated criteria pass | ✓ Good — explicit gate preserved in v1.0 |
| Defer Drift CPI (LIVE-02) | Full Drift integration required account infrastructure beyond scope | ⚠ Revisit — creates gap in RISK-01/RISK-03 enforcement |
| Telegram approval gate | Human-in-the-loop before each rebalance adds safety at cost of latency | ✓ Good — configurable timeout (default 300s) balances safety vs speed |
| `block_in_place` for proposal await | `NotifyFn` is `Box<dyn Fn>` (sync); reuses existing pattern in watch loop | ✓ Good — consistent with existing block_in_place usage |

---
*Last updated: 2026-04-10 after v1.0 milestone*
