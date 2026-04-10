# Project Retrospective

*A living document updated after each milestone. Lessons feed forward into future planning.*

## Milestone: v1.0 — MVP

**Shipped:** 2026-04-10
**Phases:** 7 | **Plans:** 20 | **Sessions:** ~4

### What Was Built
- TimescaleDB persistence layer: pool_ticks + pnl_history writes on every WebSocket tick, non-blocking via tokio::spawn
- Shadow mode with ShadowGuard, DB-logged decisions, and 2-week + zero-error live gate
- Real-data backtest engine replacing GBM simulator with TimescaleDB tick replay
- Slippage guard with tick-array walk impact computation, bps threshold, abort + log
- Orca Whirlpool live execution: close → collect → open via Anchor CPI, keypair from env var
- Risk monitor with drawdown/IL/margin-ratio thresholds, per-limit actions, DB-persisted state
- Telegram bot: rebalance proposal/approval flow, `/approve` oneshot with timeout, `/status`, `/pause`, `/resume`, `/report`

### What Worked
- Wave-based parallel execution via gsd-execute-phase: each plan ran in an isolated worktree, keeping main clean until merge
- Pure-Rust CLMM math module meant all math was independently testable before integration
- Phase dependencies were correctly ordered — no integration surprises from misordered execution
- `block_in_place` pattern for sync→async bridges was consistent and well understood by all agents
- The discuss → plan → execute → verify loop caught architectural issues before implementation began

### What Was Inefficient
- Executor agents in worktree isolation repeatedly deleted `.planning/` files from other phases (07-01-PLAN.md, 07-02-PLAN.md, 07-CONTEXT.md, 06-risk-limits plans) — required manual restoration after each Wave 2 merge
- REQUIREMENTS.md traceability table was never updated as phases completed — all 27 requirements remained "Pending" at milestone close, requiring manual reconciliation
- Phase 6 VERIFICATION.md was generated but never committed (left untracked), creating ambiguity about its canonical status
- Drift CPI (LIVE-02) was deferred mid-phase rather than scoped out upfront, creating downstream gaps in RISK-01/RISK-03 enforcement

### Patterns Established
- `block_in_place` is the standard pattern for calling sync closures (NotifyFn) from async watch loop
- `Arc<Mutex<RiskMonitor>>` shared between bot task and watch loop for operator control state
- Oneshot channel for proposal approval gate: sender held by proposal module, receiver parked with timeout
- `operator_pause` (Telegram-driven) is independent of `pause_flag` (IL-driven) — two orthogonal pause axes

### Key Lessons
1. **Explicitly instruct executor agents not to delete `.planning/` files** — they clean up aggressively when operating in worktrees and will remove plans/summaries/context from sibling phases
2. **Update REQUIREMENTS.md traceability incrementally** — doing it at milestone close is expensive; better to mark requirements complete in each phase's post-execution step
3. **Commit verification artifacts immediately** — untracked VERIFICATION.md files are invisible to git and create state ambiguity across sessions
4. **Deferred CPI scope changes need ripple-analysis** — deferring LIVE-02 mid-stream was not propagated to RISK-01/RISK-03, resulting in half-implemented enforcement paths at milestone close
5. **Stash before worktree merge when pre-existing uncommitted changes exist** — the standard worktree merge pattern needs to stash first to avoid "would be overwritten" merge failures

### Cost Observations
- Model mix: ~100% sonnet (executor + verifier agents)
- Sessions: ~4 development sessions over 7 days
- Notable: Parallel wave execution with worktrees saved significant wall-clock time but added merge overhead; for sequential phases (1→2→3 dependency chain) the overhead exceeded savings

---

## Cross-Milestone Trends

### Process Evolution

| Milestone | Sessions | Phases | Key Change |
|-----------|----------|--------|------------|
| v1.0 | ~4 | 7 | Baseline — GSD workflow established |

### Cumulative Quality

| Milestone | Rust Tests | Clippy | Zero-Dep Math |
|-----------|------------|--------|---------------|
| v1.0 | 25+ unit + property | Clean (0 warnings) | ✓ pure-Rust CLMM math |

### Top Lessons (Verified Across Milestones)

1. Instruct executor agents not to delete `.planning/` files from sibling phases
2. Update requirements traceability incrementally — not at milestone close
