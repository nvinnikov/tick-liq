# Roadmap: tick-liq

## Overview

Seven phases carry tick-liq from a CLI prototype with a scaffolded schema to a fully operational, risk-bounded LP manager running on mainnet. The sequence is strictly ordered by dependency: data must land in TimescaleDB before shadow mode can log against it, real-data backtest requires accumulated ticks, slippage guard must be in place before any live transaction fires, live execution unlocks risk limits (which guard real capital), and the Telegram bot wraps everything in a human-in-the-loop approval gate.

## Phases

- [x] **Phase 1: Persistence** - Wire pool_ticks + pnl_history writes in watch mode (completed 2026-04-09)
- [ ] **Phase 2: Shadow Mode** - Full rebalance logic running without signing or submitting transactions
- [ ] **Phase 3: Real-Data Backtest** - backtest command reads accumulated TimescaleDB ticks instead of GBM sim
- [ ] **Phase 4: Slippage Guard** - Price impact check blocks any rebalance that exceeds configured bps threshold
- [ ] **Phase 5: Live Execution** - Anchor CPI to Orca + Drift perp hedge update behind --live flag
- [ ] **Phase 6: Risk Limits** - Configurable drawdown / IL / margin-ratio thresholds with per-limit actions
- [ ] **Phase 7: Telegram Bot** - /approve blocking gate, /status, /pause, /report operator interface

## Phase Details

### Phase 1: Persistence
**Goal**: Every WebSocket tick is durably written to TimescaleDB so the system accumulates the history that all later phases depend on.
**Depends on**: Nothing (schema scaffolded in Phase 0)
**Requirements**: PERSIST-01, PERSIST-02, PERSIST-03, PERSIST-04
**Success Criteria** (what must be TRUE):
  1. Running `watch` for 10 minutes produces rows in both `pool_ticks` and `pnl_history` for every received event
  2. DB writes do not introduce measurable latency on tick processing (async, non-blocking)
  3. After a simulated WebSocket disconnect and reconnect, no duplicate rows appear (idempotent upsert on slot)
  4. Querying `pool_ticks` returns tick_current, sqrt_price, liquidity, and fee_growth_global columns populated
**Plans**: 3 plans

Plans:
- [x] 01-01: Implement async `storage::writer` module — `pool_ticks` upsert via sqlx with slot-keyed conflict resolution
- [x] 01-02: Implement `pnl_history` writer — fees_earned, il_usd, net_pnl, position_value per tick event; fire-and-forget via tokio::spawn
- [x] 01-03: Wire both writers into `watch` command event loop; integration test with embedded Postgres or test DB

### Phase 2: Shadow Mode
**Goal**: Operator can run the full rebalance decision loop for weeks without any transaction risk, building confidence and a logged decision trail before touching real capital.
**Depends on**: Phase 1
**Requirements**: SHADOW-01, SHADOW-02, SHADOW-03, SHADOW-04
**Success Criteria** (what must be TRUE):
  1. `cargo run -- watch --shadow` runs without submitting any transactions; rebalance decisions appear in logs
  2. Each shadow decision is persisted to DB with timestamp, trigger reason, price, and simulated IL delta
  3. Running `cargo run -- watch --live` without meeting the 2-week + zero-error gate returns a clear error and exits
  4. Shadow logs are queryable from DB to reconstruct full decision history
**Plans**: 4 plans

Plans:
- [ ] 02-01: Add `--shadow` / `--live` flags to CLI; implement `ShadowGuard` that blocks signing when shadow is active
- [ ] 02-02: Log shadow rebalance decisions (structured tracing + DB write to `shadow_rebalances` table)
- [ ] 02-03: Implement shadow gate check: query DB for earliest shadow_rebalance row, count errors in window; error if criteria not met
- [ ] 02-04: Integration test: verify --live rejected before 2-week threshold and accepted (mocked) after

### Phase 3: Real-Data Backtest
**Goal**: `backtest` reads actual collected tick history from TimescaleDB, replacing the GBM simulator with replay of real market microstructure.
**Depends on**: Phase 1
**Requirements**: BACKTEST-01, BACKTEST-02, BACKTEST-03
**Success Criteria** (what must be TRUE):
  1. `cargo run -- backtest --pool <ADDR> --from 2026-01-01 --to 2026-02-01` reads from pool_ticks and completes without error
  2. Output reports the same P&L metric columns as the existing GBM backtest (fees, IL, net_pnl, rebalance_count)
  3. `--from` / `--to` date range filters and strategy parameters (e.g. range width) are configurable via CLI flags
**Plans**: 3 plans

Plans:
- [ ] 03-01: Implement `storage::tick_reader` — paginated async query of pool_ticks for a pool address and date range
- [ ] 03-02: Adapt backtest engine to accept a tick stream from DB instead of GBM iterator; preserve existing output schema
- [ ] 03-03: CLI wiring + test with fixture rows inserted into test DB; verify output matches GBM baseline on synthetic data

### Phase 4: Slippage Guard
**Goal**: No rebalance transaction is ever submitted when simulated price impact exceeds the configured threshold, protecting capital from MEV and illiquid conditions.
**Depends on**: Phase 2
**Requirements**: SLIPPAGE-01, SLIPPAGE-02, SLIPPAGE-03
**Success Criteria** (what must be TRUE):
  1. Before every rebalance, simulated price impact is computed and logged
  2. A rebalance with impact above the threshold is aborted and the abort event is logged with impact bps and threshold
  3. `--max-slippage-bps` flag is accepted; default is 50 bps; value is validated at startup
**Plans**: 3 plans

Plans:
- [ ] 04-01: Implement `strategy::slippage` module — compute impact from tick array walk for a given trade size
- [ ] 04-02: Wire slippage check into rebalance decision path before any transaction construction; abort + log on breach
- [ ] 04-03: Unit tests: impact below threshold allows, above threshold aborts; CLI flag parsing and default verified

### Phase 5: Live Execution
**Goal**: The system can execute a real close → collect → open rebalance on Orca Whirlpool and update the Drift perp hedge in the same cycle, gated behind `--live` and the shadow guard.
**Depends on**: Phase 2, Phase 4
**Requirements**: LIVE-01, LIVE-02, LIVE-03, LIVE-04
**Success Criteria** (what must be TRUE):
  1. With `--live` and shadow gate satisfied, a triggered rebalance executes close → collect fees → open via Anchor CPI to Orca
  2. Drift perp hedge size is computed and logged each cycle (full Drift CPI deferred — LIVE-02 deferred)
  3. Process exits with a clear error at startup if `WALLET_KEYPAIR` env var is absent
  4. LIVE-04 atomicity (LP↔Drift rollback) deferred with LIVE-02
**Plans**: 2 plans

Plans:
- [x] 05-01-PLAN.md — Add whirlpool-cpi deps + implement OrcaExecutor with 4-step instruction builders and unit tests
- [x] 05-02-PLAN.md — Wire keypair loader + OrcaExecutor into watch loop; simulateTransaction integration tests

### Phase 6: Risk Limits
**Goal**: The running system enforces configurable hard limits on drawdown, instantaneous IL, and Drift margin ratio, taking the correct per-limit action automatically and surviving process restarts.
**Depends on**: Phase 5
**Requirements**: RISK-01, RISK-02, RISK-03, RISK-04
**Success Criteria** (what must be TRUE):
  1. When cumulative P&L drawdown exceeds `--max-drawdown`, LP position and hedge are closed and execution halts
  2. When instantaneous IL exceeds `--max-il`, rebalancing is paused but the position stays open; it resumes when IL drops
  3. When Drift margin ratio falls below `--drift-min-margin-ratio`, the Drift hedge is closed while LP remains open
  4. Risk state (peak_value, current_drawdown) is persisted to DB; limits are re-evaluated correctly after process restart
**Plans**: 3 plans

Plans:
- [x] 06-01-PLAN.md — RiskMonitor core module: RiskState, RiskAction enum, evaluate() with drawdown/IL/margin checks + unit tests
- [x] 06-02-PLAN.md — DB persistence (risk_state table, load_or_init, persist_state) + Drift User account RPC fetch
- [x] 06-03-PLAN.md — CLI flags + watch loop wiring: init, evaluate per tick, per-limit actions, state persistence

### Phase 7: Telegram Bot
**Goal**: Operator receives rebalance proposals via Telegram, can approve or let them time out, and can query status, pause, or pull a 24h P&L report at any time.
**Depends on**: Phase 5, Phase 6
**Requirements**: TG-01, TG-02, TG-03, TG-04, TG-05
**Success Criteria** (what must be TRUE):
  1. When a rebalance is triggered, a Telegram message arrives with simulated outcome; `/approve` within the timeout window allows execution
  2. If `/approve` is not received within the timeout, the rebalance is skipped and the skip is logged
  3. `/status` returns current position summary, P&L, and all three risk metrics in one message
  4. `/pause` halts rebalancing without closing positions; `/resume` restarts it; both commands are acknowledged immediately
  5. `/report` sends a summary of fees, IL, and net P&L for the trailing 24 hours
**Plans**: 3 plans

Plans:
- [ ] 07-01-PLAN.md — teloxide bot module + command router + operator_pause schema + watch loop spawn
- [ ] 07-02-PLAN.md — Rebalance proposal flow: send message, await /approve with timeout, skip+log on timeout
- [ ] 07-03-PLAN.md — /status, /pause, /resume, /report command implementations + operator_pause gate in watch loop

## Progress

**Execution Order:** 1 → 2 → 3 → 4 → 5 → 6 → 7

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Persistence | 3/3 | Complete   | 2026-04-09 |
| 2. Shadow Mode | 0/4 | Not started | - |
| 3. Real-Data Backtest | 0/3 | Not started | - |
| 4. Slippage Guard | 0/3 | Not started | - |
| 5. Live Execution | 0/2 | Not started | - |
| 6. Risk Limits | 0/3 | Not started | - |
| 7. Telegram Bot | 0/3 | Not started | - |
