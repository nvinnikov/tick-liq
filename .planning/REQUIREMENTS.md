# Requirements — Milestone v1.1 Maker Strategy Research

> **Scope:** Research-only. Zero production code changes. Deliverables are markdown reports, Dune queries, and data exports. Findings feed a strategy spec that guides future (post-v1.1) rebalancer/hedger work.

**Target pool:** `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` (Solana CLMM)

**Tooling prerequisites** (must be available before execution):
- Dune MCP server (provided by user)
- Solana CLI + Helius (or equivalent) RPC key for parsed tx history
- *(Optional)* Birdeye or DexScreener API for cross-checking pool-level time series

---

## v1.1 Requirements

### Pool Census (CENSUS)

- [ ] **CENSUS-01**: Identify the target pool's DEX (Orca Whirlpool vs Raydium CLMM), token pair, fee tier, tick spacing, and current TVL
- [ ] **CENSUS-02**: Enumerate every address that opened or held a CLMM position in the pool over the last 90 days via Dune
- [ ] **CENSUS-03**: For each LP address, record lifetime position count, active position count, cumulative liquidity provided, cumulative fees collected

### Active Maker Filter (FILTER)

- [ ] **FILTER-01**: Define quantitative criteria that distinguish *active market makers* from *passive LPs* (e.g., rebalance frequency ≥ N/week, position lifetime < X days, multiple concurrent positions)
- [ ] **FILTER-02**: Apply the filter to the CENSUS-02 address list and produce a ranked shortlist of active makers with their headline stats
- [ ] **FILTER-03**: Classify each shortlisted maker's apparent archetype (tight-range scalper, wide passive, grid/ladder, delta-hedged, etc.)

### Maker Deep-Dive (DEEP)

- [ ] **DEEP-01**: Select one maker from the FILTER-02 shortlist for in-depth analysis and justify the choice
- [ ] **DEEP-02**: Reconstruct the maker's position timeline — every open, modify, collect-fees, and close event with timestamps and tick ranges
- [ ] **DEEP-03**: Quantify rebalance cadence (distribution of inter-rebalance intervals, triggers that appear to drive rebalances)
- [ ] **DEEP-04**: Quantify range-width policy (tick-range distribution, width vs volatility relationship, recentering behaviour)
- [ ] **DEEP-05**: Measure realized fee capture (fees per $ liquidity per day) and compare to naive passive LP of the same capital
- [ ] **DEEP-06**: Detect hedging / inventory management signals on-chain (transfers to perp venues, balancing swaps) to the extent possible from public data

### Competitive Landscape (LAND)

- [ ] **LAND-01**: Segment the pool's total fees over 30/90-day windows by maker archetype (what share each archetype captures)
- [ ] **LAND-02**: Identify the top-5 fee-capturing makers and their combined share of pool fees
- [ ] **LAND-03**: Cross-reference with the equivalent Raydium CLMM pool (same token pair, closest fee tier) — is the maker population / strategy distribution materially different?

### Opportunity Sizing (SIZE)

- [ ] **SIZE-01**: Estimate the addressable fee surface (annualised $ of fees the pool distributes to LPs at current volume)
- [ ] **SIZE-02**: Estimate plausible capture rate for our target capital ($20-30k) operating the deep-dive maker's strategy, with sensitivity bands
- [ ] **SIZE-03**: Identify structural barriers (capital minimums, gas/latency disadvantages, information asymmetries) that would prevent us from matching top-quartile makers

### Strategy Specification (SPEC)

- [ ] **SPEC-01**: Consolidated findings report answering the five deliverable questions: *what to implement, surface of implementation, fees we can cover, who we compete with, our strategy*
- [ ] **SPEC-02**: Concrete rebalance policy proposal (triggers, range-width rule, cooldowns, hedging requirement yes/no) with traceability back to DEEP-* and SIZE-* evidence
- [ ] **SPEC-03**: List of open questions / follow-up research that this milestone could not answer and that gate any future implementation work

---

## Future Requirements (Deferred beyond v1.1)

- **LIVE-02**: Drift Protocol perp hedge update in the rebalance cycle (deferred from v1.0 — depends on SPEC-02 hedging verdict)
- **LIVE-04**: LP↔Drift atomicity — rollback if hedge update fails (blocked by LIVE-02)
- **RISK-01 (full)**: LP close CPI on drawdown breach (blocked by LIVE-02)
- **RISK-03 (full)**: Drift hedge close CPI on margin breach (blocked by LIVE-02)
- **E2E-01**: End-to-end integration tests with funded devnet wallet
- **IMPL-01**: Implement rebalance policy from SPEC-02 in production watch loop (new — created by v1.1)

---

## Out of Scope (v1.1)

- **Code changes to the production rebalancer, watch loop, or hedger** — explicitly deferred until after SPEC-01 is reviewed. Reason: milestone is research; implementation without findings risks anchoring on current assumptions.
- **Multi-pool management** — single pool focus preserves signal.
- **Off-chain maker attribution** (linking wallets to real-world firms) — not required for strategy replication; adds legal/privacy risk.
- **Historical pre-2025 data** — current maker behaviour is the signal, not archaeology.
- **Backtest framework changes** — existing `backtest` subcommand is sufficient to validate any SPEC-02 policy post-milestone.

---

## Traceability

Coverage: 21/21 v1.1 requirements mapped to phases 6–10. ✓

| REQ-ID     | Phase                                    | Status  |
|------------|------------------------------------------|---------|
| CENSUS-01  | Phase 6 — Pool Census                    | Pending |
| CENSUS-02  | Phase 6 — Pool Census                    | Pending |
| CENSUS-03  | Phase 6 — Pool Census                    | Pending |
| FILTER-01  | Phase 7 — Active Maker Filter            | Pending |
| FILTER-02  | Phase 7 — Active Maker Filter            | Pending |
| FILTER-03  | Phase 7 — Active Maker Filter            | Pending |
| DEEP-01    | Phase 8 — Maker Deep-Dive                | Pending |
| DEEP-02    | Phase 8 — Maker Deep-Dive                | Pending |
| DEEP-03    | Phase 8 — Maker Deep-Dive                | Pending |
| DEEP-04    | Phase 8 — Maker Deep-Dive                | Pending |
| DEEP-05    | Phase 8 — Maker Deep-Dive                | Pending |
| DEEP-06    | Phase 8 — Maker Deep-Dive                | Pending |
| LAND-01    | Phase 9 — Landscape & Opportunity Sizing | Pending |
| LAND-02    | Phase 9 — Landscape & Opportunity Sizing | Pending |
| LAND-03    | Phase 9 — Landscape & Opportunity Sizing | Pending |
| SIZE-01    | Phase 9 — Landscape & Opportunity Sizing | Pending |
| SIZE-02    | Phase 9 — Landscape & Opportunity Sizing | Pending |
| SIZE-03    | Phase 9 — Landscape & Opportunity Sizing | Pending |
| SPEC-01    | Phase 10 — Strategy Specification        | Pending |
| SPEC-02    | Phase 10 — Strategy Specification        | Pending |
| SPEC-03    | Phase 10 — Strategy Specification        | Pending |
