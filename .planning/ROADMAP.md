# Roadmap: tick-liq

## Milestones

- ✅ **v1.0 MVP** — Phases 1–7 (shipped 2026-04-10)
- 🔄 **v1.1 Maker Strategy Research** — Phases 6–10 (research-only; no production code changes)

## Phases

<details>
<summary>✅ v1.0 MVP (Phases 1–7) — SHIPPED 2026-04-10</summary>

- [x] Phase 1: Persistence (3/3 plans) — completed 2026-04-09
- [x] Phase 2: Shadow Mode (3/3 plans) — completed 2026-04-09
- [x] Phase 3: Real-Data Backtest (3/3 plans) — completed 2026-04-09
- [x] Phase 4: Slippage Guard (3/3 plans) — completed 2026-04-10
- [x] Phase 5: Live Execution (2/2 plans) — completed 2026-04-10
- [x] Phase 6: Risk Limits (3/3 plans) — completed 2026-04-10 ⚠ LP/Drift close CPIs deferred (LIVE-02 tech debt)
- [x] Phase 7: Telegram Bot (3/3 plans) — completed 2026-04-10

Full archive: `.planning/milestones/v1.0-ROADMAP.md`

</details>

### v1.1 Maker Strategy Research

> **Starts with one infrastructure phase (11), then research-only.** Deliverables are markdown reports, Dune queries, and data exports under `.planning/research/v1.1/`. Only Phase 11 touches `src/`.

- [ ] **Phase 11: CEX price feed via Binance WebSocket** — replace on-chain pool price with independent CEX feed
- [ ] **Phase 6: Pool Census** — enumerate every LP address on the target pool with lifetime stats
- [ ] **Phase 7: Active Maker Filter** — define maker criteria, apply to census, classify archetypes
- [ ] **Phase 8: Maker Deep-Dive** — pick one maker, reconstruct timeline, quantify cadence/width/fees/hedging signals
- [ ] **Phase 9: Landscape & Opportunity Sizing** — segment pool fees by archetype, cross-check Raydium, size addressable fees and our capture
- [ ] **Phase 10: Strategy Specification** — consolidate findings into spec, rebalance policy, and open-questions list

## Phase Details

### Phase 11: CEX price feed via Binance WebSocket
**Goal**: Replace on-chain pool price (tick_current_index) with an independent Binance WebSocket feed so the price used for rebalance decisions and IL calculation is not sourced from the pool being market-made.
**Depends on**: Nothing (first phase of v1.1).
**Requirements**: TBD
**Plans**: 2 plans
- [x] 11-01-PLAN.md — Binance bookTicker WS module (src/data/cex_ws.rs) with parser, reconnect loop, shared state; unit tests
- [ ] 11-02-PLAN.md — Wire --cex-symbol CLI flag, spawn Binance task, resolve CEX-or-fallback price in watch loop; human-verify live feed

### Phase 6: Pool Census
**Goal**: Produce a complete, authoritative list of every address that has provided liquidity on pool `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` in the last 90 days, with per-address lifetime stats.
**Depends on**: Nothing (first phase of v1.1). Requires Dune MCP and Helius RPC available.
**Requirements**: CENSUS-01, CENSUS-02, CENSUS-03
**Success Criteria** (what must be TRUE):
  1. `.planning/research/v1.1/CENSUS.md` exists and identifies the pool's DEX, token pair, fee tier, tick spacing, and current TVL with sources cited.
  2. A Dune query (saved URL + SQL in the report) enumerates every LP address active in the last 90 days, with the raw export committed under `.planning/research/v1.1/data/census-addresses.csv`.
  3. The report contains a per-address table with lifetime position count, active position count, cumulative liquidity provided, and cumulative fees collected — non-empty for every address in the export.
  4. A reviewer can reproduce the census by running the committed Dune query against the target pool and getting byte-identical address set.
**Plans**: TBD

### Phase 7: Active Maker Filter
**Goal**: Separate active market makers from passive LPs, rank them, and classify each by apparent strategy archetype.
**Depends on**: Phase 6
**Requirements**: FILTER-01, FILTER-02, FILTER-03
**Success Criteria** (what must be TRUE):
  1. `.planning/research/v1.1/FILTER.md` defines explicit quantitative maker criteria (thresholds for rebalance frequency, position lifetime, concurrent positions) with numeric justification.
  2. The criteria applied to the Phase 6 census produce a ranked shortlist of active makers with headline stats (fees, rebalances, range widths) — export saved to `.planning/research/v1.1/data/active-makers.csv`.
  3. Each shortlisted maker is tagged with an archetype label (tight-range scalper, wide passive, grid/ladder, delta-hedged, other) and a one-line evidence snippet supporting the label.
  4. The filter's sensitivity is documented: how many makers survive at ±1 threshold step for each criterion.
**Plans**: TBD

### Phase 8: Maker Deep-Dive
**Goal**: For one chosen maker, produce a fully reconstructed on-chain record and quantitative characterisation of their strategy — the behavioural template we may replicate.
**Depends on**: Phase 7
**Requirements**: DEEP-01, DEEP-02, DEEP-03, DEEP-04, DEEP-05, DEEP-06
**Success Criteria** (what must be TRUE):
  1. `.planning/research/v1.1/DEEP-DIVE.md` names the chosen maker address and gives a written justification for the selection referencing Phase 7 rankings.
  2. The report includes the full position timeline (every open, modify, collect-fees, close event with timestamps and tick ranges) for that address, with raw event export at `.planning/research/v1.1/data/deep-dive-events.csv`.
  3. Rebalance cadence is quantified with a distribution plot/table of inter-rebalance intervals plus a short list of apparent triggers (price moves, time-of-day, volatility regimes).
  4. Range-width policy is quantified: width distribution, width-vs-volatility scatter/regression, and documented recentering behaviour (how close to mid-price ranges are re-anchored).
  5. Realized fee capture (fees per $ liquidity per day) is computed and compared side-by-side to a naive passive-LP counterfactual on the same capital and window, with hedging / inventory-management signals (perp transfers, balancing swaps) either documented or explicitly marked "no signal found."
**Plans**: TBD

### Phase 9: Landscape & Opportunity Sizing
**Goal**: Place the deep-dive maker in context — who else is making the pool, what share of fees each archetype captures, how the pool compares to Raydium, and how much of the fee surface is realistically addressable at $20–30k capital.
**Depends on**: Phase 7 (needs archetype labels), Phase 8 (needs deep-dive strategy baseline for capture estimate)
**Requirements**: LAND-01, LAND-02, LAND-03, SIZE-01, SIZE-02, SIZE-03
**Success Criteria** (what must be TRUE):
  1. `.planning/research/v1.1/LANDSCAPE.md` shows pool fee share by maker archetype over both 30-day and 90-day windows, plus a named top-5 fee-capturing makers with their combined share.
  2. The report cross-references the equivalent Raydium CLMM pool (same token pair, closest fee tier) and states a defensible verdict on whether maker population / strategy distribution differs materially, with supporting numbers.
  3. `.planning/research/v1.1/SIZING.md` gives an annualised addressable fee surface for the target pool derived from current volume, with the arithmetic shown.
  4. Sizing report provides a plausible capture-rate estimate for $20–30k capital operating the Phase 8 deep-dive strategy, including sensitivity bands (pessimistic / base / optimistic) and the assumptions driving each band.
  5. A named list of structural barriers (capital minimums, gas/latency disadvantages, information asymmetries) is given, each annotated with "blocks us", "limits us", or "does not apply".
**Plans**: TBD

### Phase 10: Strategy Specification
**Goal**: Collapse the research into a single actionable spec that tells future implementation work exactly what to build, what fees it can cover, who it competes with, and how it rebalances.
**Depends on**: Phases 6, 7, 8, 9
**Requirements**: SPEC-01, SPEC-02, SPEC-03
**Success Criteria** (what must be TRUE):
  1. `.planning/research/v1.1/STRATEGY-SPEC.md` answers the five deliverable questions in five clearly-labelled sections: what to implement, implementation surface, fees we can cover, who we compete with, our strategy.
  2. The spec contains a concrete rebalance policy proposal (triggers, range-width rule, cooldowns, hedging-required yes/no) — every parameter is cited back to a specific DEEP-* or SIZE-* finding.
  3. An "Open Questions" section enumerates every unresolved issue this milestone could not answer and marks each as a gate for a specific future workstream (e.g., "gates IMPL-01", "gates LIVE-02 go/no-go").
  4. The spec is self-contained: a reader with zero milestone context can, after reading only STRATEGY-SPEC.md, state the recommended strategy, expected fee capture, and the conditions that would invalidate it.
**Plans**: TBD

## Progress

| Phase | Milestone | Plans | Status | Completed |
|-------|-----------|-------|--------|-----------|
| 1. Persistence | v1.0 | 3/3 | Complete | 2026-04-09 |
| 2. Shadow Mode | v1.0 | 3/3 | Complete | 2026-04-09 |
| 3. Real-Data Backtest | v1.0 | 3/3 | Complete | 2026-04-09 |
| 4. Slippage Guard | v1.0 | 3/3 | Complete | 2026-04-10 |
| 5. Live Execution | v1.0 | 2/2 | Complete | 2026-04-10 |
| 6. Risk Limits | v1.0 | 3/3 | Complete (partial) | 2026-04-10 |
| 7. Telegram Bot | v1.0 | 3/3 | Complete | 2026-04-10 |
| 11. CEX price feed via Binance WebSocket | v1.1 | 1/2 | In Progress|  |
| 6. Pool Census | v1.1 | 0/0 | Not started | - |
| 7. Active Maker Filter | v1.1 | 0/0 | Not started | - |
| 8. Maker Deep-Dive | v1.1 | 0/0 | Not started | - |
| 9. Landscape & Opportunity Sizing | v1.1 | 0/0 | Not started | - |
| 10. Strategy Specification | v1.1 | 0/0 | Not started | - |
