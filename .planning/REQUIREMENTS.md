# Requirements: tick-liq

**Defined:** 2026-04-09
**Core Value:** Profitable, hands-off LP with automated rebalancing and delta hedge — verifiable in shadow before any capital is at risk.

## v1 Requirements

### Persistence (TimescaleDB Writes)

- [ ] **PERSIST-01**: `watch` command writes pool state snapshot (tick_current, sqrt_price, liquidity, fee_growth_global) to `pool_ticks` on every WebSocket event
- [ ] **PERSIST-02**: `watch` command writes P&L delta (fees_earned, il_usd, net_pnl, position_value) to `pnl_history` on every WebSocket event
- [ ] **PERSIST-03**: DB writes are non-blocking — tick processing latency unaffected by storage I/O
- [ ] **PERSIST-04**: Watcher reconnects after WebSocket disconnect with no duplicate rows (idempotent upsert on slot)

### Shadow Mode

- [ ] **SHADOW-01**: `--shadow` flag on `watch` runs full rebalance decision logic without signing or submitting transactions
- [ ] **SHADOW-02**: Each shadow rebalance decision is logged (timestamp, trigger reason, price, simulated IL delta)
- [ ] **SHADOW-03**: Shadow rebalance log is persisted to DB (`shadow_rebalances` table or equivalent)
- [ ] **SHADOW-04**: Live trading requires explicit `--live` flag — shadow is the default; `--live` without 2-week runtime + zero errors returns an error

### Real-Data Backtest

- [ ] **BACKTEST-01**: `backtest` command reads tick history from `pool_ticks` via TimescaleDB for a given pool address and date range
- [ ] **BACKTEST-02**: Produces same P&L metrics as existing GBM simulator (fees, IL, net, rebalance count)
- [ ] **BACKTEST-03**: Configurable date range (`--from`, `--to`) and strategy parameters

### Slippage Guard

- [ ] **SLIPPAGE-01**: Rebalance checks simulated price impact before any transaction submission
- [ ] **SLIPPAGE-02**: Transaction is aborted if simulated slippage exceeds threshold; event logged
- [ ] **SLIPPAGE-03**: Threshold configurable via `--max-slippage-bps` CLI flag (default: 50bps)

### Live Execution

- [ ] **LIVE-01**: Rebalance executes close → collect fees → open sequence via Anchor CPI to Orca Whirlpool program
- [ ] **LIVE-02**: Drift Protocol perp hedge is updated in the same rebalance cycle (size = position delta)
- [ ] **LIVE-03**: Keypair loaded exclusively from env var (`WALLET_KEYPAIR`); process exits if var absent
- [ ] **LIVE-04**: Rebalance is atomic: if Drift hedge update fails, LP rebalance is rolled back (or vice versa)

### Risk Limits

- [ ] **RISK-01**: `--max-drawdown <pct>` — when cumulative P&L drawdown exceeds threshold, closes LP position and hedge; halts further execution
- [ ] **RISK-02**: `--max-il <pct>` — when instantaneous IL exceeds threshold, pauses rebalancing (position stays open)
- [ ] **RISK-03**: `--drift-min-margin-ratio <pct>` — when Drift margin ratio falls below threshold, closes Drift hedge only (LP stays open)
- [ ] **RISK-04**: Risk state (current drawdown, peak value) is persisted in DB and survives process restart

### Telegram Bot

- [ ] **TG-01**: Bot sends rebalance proposal message with simulated outcome; waits for `/approve` up to configurable timeout
- [ ] **TG-02**: Unapproved rebalance is skipped after timeout; event logged
- [ ] **TG-03**: `/status` returns current position summary, P&L, and risk metrics
- [ ] **TG-04**: `/pause` halts rebalancing without closing positions; `/resume` restarts
- [ ] **TG-05**: `/report` sends last 24h P&L summary

## v2 Requirements

### Advanced Execution

- **EXEC-01**: Jito bundle integration for atomic multi-instruction rebalance (MEV protection)
- **EXEC-02**: Multi-pool simultaneous management with shared risk budget
- **EXEC-03**: Fee-optimized routing (compare Orca vs Raydium for same pair)

### Analytics

- **ANAL-01**: Web dashboard with P&L charts and position history
- **ANAL-02**: Historical tick import from Birdeye/Flipside for pre-watcher backtest

## Out of Scope

| Feature | Reason |
|---------|--------|
| Jito bundles (v1) | Slippage guard sufficient at $20-30k capital; add in v2 |
| Historical tick import | Shadow window provides enough data; import complexity not justified |
| Multi-pool (v1) | Single-pool focus for shadow validation; multi-pool in v2 |
| Web dashboard | CLI + Telegram sufficient for operator UX |
| Token accounts beyond SOL/USDC, SOL/USDT | Scope locked to 5bps pools for initial deployment |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| PERSIST-01 | Phase 1 | Pending |
| PERSIST-02 | Phase 1 | Pending |
| PERSIST-03 | Phase 1 | Pending |
| PERSIST-04 | Phase 1 | Pending |
| SHADOW-01 | Phase 2 | Pending |
| SHADOW-02 | Phase 2 | Pending |
| SHADOW-03 | Phase 2 | Pending |
| SHADOW-04 | Phase 2 | Pending |
| BACKTEST-01 | Phase 3 | Pending |
| BACKTEST-02 | Phase 3 | Pending |
| BACKTEST-03 | Phase 3 | Pending |
| SLIPPAGE-01 | Phase 4 | Pending |
| SLIPPAGE-02 | Phase 4 | Pending |
| SLIPPAGE-03 | Phase 4 | Pending |
| LIVE-01 | Phase 5 | Pending |
| LIVE-02 | Phase 5 | Pending |
| LIVE-03 | Phase 5 | Pending |
| LIVE-04 | Phase 5 | Pending |
| RISK-01 | Phase 6 | Pending |
| RISK-02 | Phase 6 | Pending |
| RISK-03 | Phase 6 | Pending |
| RISK-04 | Phase 6 | Pending |
| TG-01 | Phase 7 | Pending |
| TG-02 | Phase 7 | Pending |
| TG-03 | Phase 7 | Pending |
| TG-04 | Phase 7 | Pending |
| TG-05 | Phase 7 | Pending |

**Coverage:**
- v1 requirements: 27 total
- Mapped to phases: 27
- Unmapped: 0 ✓

---
*Requirements defined: 2026-04-09*
*Last updated: 2026-04-09 after initial definition*
