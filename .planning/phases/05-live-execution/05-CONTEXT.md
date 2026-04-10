# Phase 5: Live Execution - Context

**Gathered:** 2026-04-10
**Status:** Ready for planning
**Source:** /gsd-discuss-phase 5

<domain>
## Phase Boundary

Phase 5 wires real Orca Whirlpool close → collect fees → open_position transactions via Anchor CPI, gated behind `--live` and the Phase 2 shadow guard. The Drift perp hedge is computed but **not submitted** (deferred). Phase 5 is the capital-safety-critical path; Drift full integration is out of scope.

</domain>

<decisions>
## Implementation Decisions

### Orca CPI approach (LOCKED)
- Add `whirlpool` Anchor client crate (git, with `cpi` feature) + `anchor-client = "0.29"` + `anchor-lang = "0.29"`
- Use IDL-generated CPI instruction builders: `whirlpool::cpi::close_position`, `collect_fees`, `open_position`
- Consistent with Anchor 0.29 already referenced in CLAUDE.md
- Pattern: build `CpiContext` with the appropriate accounts, call cpi fn, propagate errors

### Drift hedge in Phase 5 (LOCKED)
- Drift perp hedge is **stub only** in Phase 5
- `hedge.rs::compute_hedge_size()` already computes the correct size
- Phase 5 adds a `tracing::info!` log of the computed size but does NOT submit any Drift instruction
- LIVE-02 ("Drift perp hedge updated in same cycle") is **deferred to a later phase**
- LIVE-04 ("atomicity between LP and hedge") is **deferred with LIVE-02** — vacuously satisfied when Drift is a stub

### Atomicity model for Orca sequence (LOCKED)
- Sequence: `close_position` → `collect_fees` → `open_position` (three separate Solana transactions)
- On `open_position` failure: **retry once** (2-second delay), then **halt**
- On halt: emit `tracing::error!` at CRITICAL level with exact failure point and partial state
- Persist failure to `shadow_rebalances` row with `error_flag = true`, `trigger_reason = 'live_rebalance_failed'`, `error_message` = the error string
- No auto-recovery at current price — operator must manually re-open via CLI
- `close_position` and `collect_fees` do not retry (idempotent-safe; retry of close on already-closed account = account not found error → halt immediately)

### Error persistence (LOCKED)
- Reuse existing `shadow_rebalances` table for live execution failure rows
- `trigger_reason` values for live mode: `'live_rebalance_ok'` (success), `'live_rebalance_failed'` (failure)
- `error_flag = true` on any step failure; `error_message` = formatted anyhow error chain
- Live success rows fill `simulated_range_width`, `simulated_fees_earned`, `simulated_il_usd`, `simulated_net_pnl` with the actual observed values post-execution (or best available pre-execution estimate)

### Keypair loading (LOCKED — from REQUIREMENTS.md LIVE-03)
- `WALLET_KEYPAIR` env var holds the keypair as a JSON byte array (standard Solana CLI format: `[u8; 64]`)
- Process exits at startup with a clear error if `WALLET_KEYPAIR` is absent
- `solana_sdk::signer::keypair::Keypair::from_bytes()` is the loader
- No file path option, no config file option

### Test strategy (LOCKED)
- No funded devnet wallet required for Phase 5 completion
- Tests use `simulateTransaction` RPC call — validates instruction serialization and account derivation without consuming funds or modifying state
- Tests marked `#[ignore]` by default, enabled with `cargo test -- --include-ignored` when `WALLET_KEYPAIR` and `RPC_URL` env vars are set
- Minimum CI bar: `cargo build` passes + `simulateTransaction` round-trip passes for close/collect/open sequence

### Claude's Discretion
- Exact account derivation helpers (PDA seeds for position NFT mint, tick array PDAs, token vaults)
- CpiContext account struct layout (which accounts to pass for each instruction)
- Whether to extract an `OrcaExecutor` struct or keep CPI calls inline in main.rs
- Tracing span structure for the live rebalance cycle

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Existing execution stubs (replace these)
- `src/execution/rebalance.rs` — `build_rebalance_plan()` is dry-run; Phase 5 replaces the submission path
- `src/execution/hedge.rs` — `compute_hedge_size()` stays, Phase 5 adds tracing::info! log of the size, no CPI
- `src/execution/shadow_guard.rs` — `ShadowGuard::Live::submit()` is the placeholder; Phase 5 wires real Orca CPI here

### Existing protocol layer (extend this)
- `src/protocols/orca.rs` — `WhirlpoolPool`, `WhirlpoolPosition` structs; Phase 5 adds instruction builders alongside

### Storage layer (extend this)
- `src/storage/writer.rs` — `ShadowRebalanceRow`, `spawn_shadow_write`; Phase 5 adds live result rows with same type
- `src/storage/schema.sql` — no schema changes needed; `shadow_rebalances` table already supports `trigger_reason = 'live_rebalance_failed'`

### Main watch loop (integration point)
- `src/main.rs` — slippage gate + shadow guard already wired; Phase 5 replaces the `ShadowGuard::Live` no-op with real Orca CPI calls

### Requirements
- `.planning/REQUIREMENTS.md` — LIVE-01, LIVE-03 (active); LIVE-02, LIVE-04 (deferred)

### External
- Orca Whirlpool program reference: `src/protocols/orca.rs` (WHIRLPOOL_PROGRAM_ID constant, existing pool/position structs)

</canonical_refs>

<specifics>
## Specific Implementation Notes

- Drift is a stub in Phase 5 — `tracing::info!(size_usd, side, "drift hedge size computed (not submitted)")`. Do not add any Drift deps.
- Three separate Solana txns (close, collect, open) — not combined into one. CU budget for a combined tx is too tight.
- `open_position` retry: wait 2 seconds, retry with identical parameters, halt on second failure
- Failure path writes to `shadow_rebalances` with `error_flag=true` — this will cause the shadow gate to fail for any subsequent `--live` attempt, which is intentional safety behavior
- `simulateTransaction` tests should cover: (1) close_position IX valid, (2) collect_fees IX valid, (3) open_position IX valid for a test range. Assert no simulation error and no missing accounts.

</specifics>

<deferred>
## Deferred Ideas

- Drift perp hedge real CPI (LIVE-02) — deferred to a later phase, add Drift deps then
- LIVE-04 atomicity between LP and Drift hedge — deferred with LIVE-02
- Devnet integration test (funded wallet roundtrip) — noted for v2 or manual verification
- Jito bundle for MEV-protected rebalance — v2 scope (EXEC-01 in REQUIREMENTS.md)

</deferred>

---

*Phase: 05-live-execution*
*Context gathered: 2026-04-10 via /gsd-discuss-phase 5*
