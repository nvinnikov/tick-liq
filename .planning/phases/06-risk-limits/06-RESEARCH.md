# Phase 6: Risk Limits - Research

**Researched:** 2026-04-10
**Domain:** Rust state machine design, PostgreSQL upsert patterns, Drift Protocol account deserialization
**Confidence:** HIGH

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

**Drift margin ratio source**
- D-01: Phase 6 adds a real read-only RPC fetch of the Drift User account to get the actual margin ratio — NOT a stub.
- D-02: No Drift CPI is added in Phase 6. This is monitoring only (read + evaluate + log/act), not execution.
- D-03: Crate vs. manual borsh layout for Drift account deserialization = Claude's discretion (prefer official crate if it avoids brittle offsets).

**Risk check timing**
- D-04: Risk monitor runs on every incoming WebSocket tick.
- D-05: Evaluation order: `pnl_history write` → risk check → `should_rebalance()` → slippage → execute.
- D-06: A breach halts the rest of the tick cycle immediately (no rebalance evaluation on a breached tick).

**IL pause / auto-resume**
- D-07: IL pause is fully automatic. `pause_flag` set in DB risk_state row when IL exceeds `--max-il`.
- D-08: IL auto-resumes when IL drops back below `--max-il`. `pause_flag` cleared automatically.
- D-09: No hysteresis. Resume threshold = pause threshold = `--max-il`. Oscillation is correct behavior.

**Drawdown close-all scope**
- D-10: Drawdown breach closes LP position only via `OrcaExecutor` (`close_position` + `collect_fees`). Drift hedge close NOT attempted; emit `tracing::error!` at CRITICAL: `"halt: drawdown limit hit — Drift hedge close deferred (LIVE-02)"`.
- D-11: After LP close, process does NOT exit. Sets `halt_flag = true` in DB and continues running.
- D-12: `halt_flag` survives restart. Operator must manually clear via SQL UPDATE. Restart does not auto-clear.

### Claude's Discretion
- Exact Rust struct layout for `RiskState` in-memory representation
- DB schema columns for `risk_state` table (required minimum: `pool_address`, `peak_pnl`, `current_drawdown_pct`, `pause_flag`, `halt_flag`, `updated_at`)
- Crate choice for Drift account deserialization (prefer official crate if it avoids brittle offsets; fall back to borsh layout if crate adds too many transitive deps)
- Tracing span structure for risk breaches
- Where to add `--max-drawdown`, `--max-il`, `--drift-min-margin-ratio` CLI args (add to `Commands::Watch` variant alongside `--max-slippage-bps`)

### Deferred Ideas (OUT OF SCOPE)
- Drift hedge close execution (RISK-03 action) — requires LIVE-02 Drift CPI
- LIVE-04 atomicity between LP close and Drift hedge close
- Telegram `/resume` command to clear halt_flag (Phase 7)
- CLI `watch --clear-halt` flag (potential Phase 7)
</user_constraints>

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| RISK-01 | `--max-drawdown <pct>` — when cumulative P&L drawdown exceeds threshold, closes LP position and hedge; halts further execution | `close_all` action via OrcaExecutor; `halt_flag` in DB |
| RISK-02 | `--max-il <pct>` — when instantaneous IL exceeds threshold, pauses rebalancing (position stays open) | `pause_flag` in DB; auto-resume on recovery |
| RISK-03 | `--drift-min-margin-ratio <pct>` — when Drift margin ratio falls below threshold, closes Drift hedge only (LP stays open) | RPC fetch of Drift User account; monitoring-only, no CPI |
| RISK-04 | Risk state (current drawdown, peak value) is persisted in DB and survives process restart | `risk_state` table; upsert on every tick; load at startup |
</phase_requirements>

---

## Summary

Phase 6 adds a `strategy::risk_monitor` module that implements three independent circuit-breaker types evaluated on every WebSocket tick, in order, before `should_rebalance()` is called. The module is a pure state machine: it reads a `PnlSnapshot` from the tick's already-computed P&L values, compares against configurable thresholds, emits a `RiskAction` variant, and persists updated state to a new `risk_state` DB table. No new async I/O patterns are required — the existing `tokio::spawn` fire-and-forget DB write pattern (established in Phase 1) handles state persistence without blocking tick processing.

The most novel element is Drift User account deserialization for RISK-03. Research confirms that computing a true margin ratio off-chain requires oracle context that is impractical to replicate in Phase 6. The recommended approach is to use the `drift-rs` IDL-generated type structs for deserialization via borsh, then compute a simplified proxy metric from the available account fields (total_collateral from spot positions, maintenance_margin from perp position notionals), with explicit documentation of the approximation. The full `drift-rs` crate is too heavy (gRPC, `libdrift` native library, yellowstone-grpc deps) for a read-only monitoring use case; a lightweight borsh-based manual deserialization of the subset of fields needed is the correct tradeoff.

The four plans align cleanly with the codebase: (1) `RiskMonitor` struct and `evaluate()` method with all three checks, (2) three action handlers calling existing executor methods, (3) `risk_state` DB table and startup load, (4) CLI args and unit tests.

**Primary recommendation:** Implement `RiskMonitor` as a struct holding in-memory `RiskState` (loaded from DB at startup), evaluate all three limits on each tick, persist the updated state via fire-and-forget spawn, and return a `RiskAction` enum that main.rs uses to gate the rest of the tick cycle. Use manual borsh deserialization for Drift User account — not the full `drift-rs` crate.

---

## Standard Stack

### Core (no new dependencies required)

All dependencies needed for Phase 6 are already present in `Cargo.toml`:

| Library | Version (Cargo.toml) | Purpose in Phase 6 |
|---------|---------------------|---------------------|
| `sqlx-core` | 0.8 | DB upsert for `risk_state` table |
| `sqlx-postgres` | 0.8 | PgPool for DB writes |
| `borsh` | 0.10 | Drift User account deserialization |
| `solana-client` | 1.18 | RPC `get_account_data` for Drift User account |
| `tokio` | 1 (full) | fire-and-forget spawn for DB writes |
| `tracing` | 0.1 | Structured CRITICAL-level breach logging |
| `anyhow` | 1 | Error handling per project convention |
| `clap` | 4 (derive, env) | Three new CLI flags on `Commands::Watch` |

[VERIFIED: codebase grep of Cargo.toml]

**No new crates needed.** The decision to use manual borsh deserialization for the Drift User account (rather than the full `drift-rs` or `drift-cpi` crate) avoids adding heavyweight transitive dependencies (gRPC, `yellowstone-grpc-*`, `libdrift` native library).

### Why Not Use drift-rs

`drift-rs` v1.0.0 (released March 2026) requires:
- `yellowstone-grpc-client` and `yellowstone-grpc-proto` (gRPC transport for market subscriptions)
- `libdrift` native library installed locally
- `solana-rpc-client` (may conflict with project's `solana-client` 1.18)

For Phase 6's read-only monitoring purpose (one RPC `get_account_data` call per tick), these transitive dependencies are not justified. [VERIFIED: docs.rs/drift-rs/latest + GitHub README]

---

## Architecture Patterns

### Module Structure

```
src/strategy/
├── mod.rs          # add: pub mod risk_monitor + re-export RiskMonitor, RiskState, RiskAction
├── risk_monitor.rs # new: all risk logic
├── signal.rs       # unchanged
└── slippage.rs     # unchanged
```

No new files needed in `execution/` or `storage/` beyond the schema addition.

### Pattern 1: RiskState In-Memory Struct

**What:** A plain struct holding all persisted state fields; mirrors the `risk_state` DB row.

**When to use:** Instantiated once at watch startup (loaded from DB or initialized fresh). Mutated in-place on each tick. Written to DB via fire-and-forget spawn.

```rust
// Source: derived from CONTEXT.md § DB schema requirements + established project patterns
#[derive(Debug, Clone)]
pub struct RiskState {
    pub pool_address: String,
    pub peak_pnl: f64,
    pub current_drawdown_pct: f64,
    pub pause_flag: bool,   // IL limit active
    pub halt_flag: bool,    // drawdown limit fired; rebalancing permanently halted
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

### Pattern 2: RiskAction Enum

**What:** Discriminated return type from `RiskMonitor::evaluate()`, consumed in main.rs to decide what to do on the current tick.

**When to use:** Replaces ad-hoc boolean flags. Each variant carries the breach type for logging.

```rust
// Source: derived from CONTEXT.md locked decisions D-06 through D-11
#[derive(Debug, Clone, PartialEq)]
pub enum RiskAction {
    /// All limits OK — proceed with normal tick cycle.
    Continue,
    /// IL limit breached (or was already breached and IL is still high).
    /// Rebalancing paused. Position stays open. Auto-resumes when IL clears.
    PauseRebalancing { il_pct: f64 },
    /// IL was previously paused, now recovered. Resume rebalancing.
    ResumeRebalancing { il_pct: f64 },
    /// Drawdown limit breached. LP close attempted. halt_flag set. Halt permanently.
    HaltAll { drawdown_pct: f64 },
    /// Drift margin ratio below threshold. Log CRITICAL. No CPI (deferred LIVE-02).
    CloseDriftHedge { margin_ratio: f64 },
}
```

### Pattern 3: RiskMonitor Struct

**What:** Stateful evaluator. Holds in-memory `RiskState` and the three threshold values.

```rust
// Source: derived from CONTEXT.md locked decisions
pub struct RiskMonitor {
    state: RiskState,
    max_drawdown_pct: Option<f64>,       // None = limit disabled (flag not set)
    max_il_pct: Option<f64>,
    drift_min_margin_ratio: Option<f64>,
    drift_user_pubkey: Option<solana_sdk::pubkey::Pubkey>,
    rpc_url: String,
}

impl RiskMonitor {
    pub async fn evaluate(&mut self, snap: &PnlSnapshot) -> anyhow::Result<RiskAction>;
    pub async fn load_or_init(pool: &PgPool, pool_address: &str) -> anyhow::Result<RiskState>;
    pub fn persist_state(pool: PgPool, state: RiskState);  // fire-and-forget spawn
}
```

### Pattern 4: Evaluation Order (D-05, D-06)

Evaluation must stop at the first breach and return immediately — no need to check all three if drawdown fires first.

```
evaluate(&snap) {
    1. If halt_flag already set → return HaltAll immediately (no limit check needed)
    2. Drawdown check (most destructive) → if breached: trigger close_all, set halt_flag, return HaltAll
    3. IL check (pause/resume) → if breached: set pause_flag, return PauseRebalancing
                                  if recovered: clear pause_flag, return ResumeRebalancing
    4. Drift margin check → if below threshold: log CRITICAL, return CloseDriftHedge
    5. Return Continue
}
```

**Critical:** Step 1 must handle the restart case. When `halt_flag = true` is loaded from DB at startup, the process continues running but must suppress rebalancing on every subsequent tick. The caller (main.rs) handles this by checking `RiskAction::HaltAll` or, more ergonomically, by inspecting `risk_monitor.state.halt_flag` directly before the rebalance path.

### Pattern 5: DB Upsert for risk_state

**What:** PostgreSQL `INSERT ... ON CONFLICT (pool_address) DO UPDATE` — same idempotency pattern as `pool_ticks` but for a single-row-per-pool state table.

**When to use:** After each `evaluate()` call that mutates state.

```sql
-- Source: derived from existing pool_ticks idempotency pattern [VERIFIED: codebase]
INSERT INTO risk_state
  (pool_address, peak_pnl, current_drawdown_pct, pause_flag, halt_flag, updated_at)
VALUES ($1, $2, $3, $4, $5, NOW())
ON CONFLICT (pool_address)
DO UPDATE SET
  peak_pnl = EXCLUDED.peak_pnl,
  current_drawdown_pct = EXCLUDED.current_drawdown_pct,
  pause_flag = EXCLUDED.pause_flag,
  halt_flag = EXCLUDED.halt_flag,
  updated_at = EXCLUDED.updated_at;
```

### Pattern 6: Drift User Account — Manual Borsh Deserialization

**What:** Fetch the Drift User account via `solana_client::rpc_client::RpcClient::get_account_data`, skip the 8-byte Anchor discriminator, deserialize the partial layout via borsh to extract the fields needed for a proxy margin ratio.

**The constraint:** True Drift margin ratio requires oracle prices for all perp and spot markets in the account. These are separate on-chain accounts (PerpMarket, SpotMarket, oracles) that change every slot. Replicating this full calculation off-chain in Phase 6 would require fetching 10-20 additional accounts per tick — impractical and out of scope.

**The recommended approach (RISK-03):** Compute a simplified proxy metric that is directionally correct for risk monitoring:
- Fetch Drift User account raw bytes via RPC
- Skip 8-byte discriminator (Anchor accounts always have this prefix)
- Deserialize only the fields needed: `settled_perp_pnl` (i64, cumulative), `perp_positions` (array of `PerpPosition` structs with `base_asset_amount` and `quote_asset_amount`)
- Compute: `notional_exposure = sum(|base_asset_amount| * current_price)` using the tick's `price_current`
- Proxy margin ratio = `quote_asset_amount_sum / notional_exposure` (approximation; no oracle weighting)
- If RPC fetch fails: log warning, treat as "margin ratio OK" (D-03 fallback, per CONTEXT.md specifics)

This is explicitly an approximation, must be documented as such in code, and is sufficient for Phase 6 monitoring. Full oracle-aware calculation is LIVE-02 scope.

[ASSUMED] The Drift User account discriminator is 8 bytes (standard Anchor anchor-lang discriminator). If Drift uses a non-standard layout, this will fail at runtime and fall into the error-handling path (treat as OK + log warning).

### Pattern 7: DRIFT_USER_PUBKEY Derivation

Drift User PDAs are derived as: `PDA(["user", authority_pubkey, subaccount_index])` where `subaccount_index = 0` for the default subaccount. The authority is the same wallet keypair used for LP management. [CITED: https://github.com/drift-labs/protocol-v2]

```rust
// Drift User PDA derivation (subaccount 0)
// Program ID: dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH (mainnet)
let drift_program_id = solana_sdk::pubkey!("dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH");
let (user_pda, _) = solana_sdk::pubkey::Pubkey::find_program_address(
    &[b"user", authority.as_ref(), &[0u8, 0u8]], // subaccount 0
    &drift_program_id,
);
```

[ASSUMED] Drift mainnet program ID above is correct as of training data. Should be verified against official Drift documentation before deployment.

### Anti-Patterns to Avoid

- **Blocking tick on DB write:** DB upsert for `risk_state` must use `tokio::spawn` fire-and-forget, same as `spawn_pnl_write`. Never `.await` a DB write inline in the tick callback.
- **Blocking tick on RPC:** Drift User RPC fetch is a network call. It must be done with a timeout. Use `SolanaRpc::with_timeout(&rpc_url, rpc_timeout)` already wired in the watch arm, not an unbounded RpcClient.
- **Drift fetch failure = halt:** Per CONTEXT.md specifics, if Drift RPC fails, treat as "margin OK" and log a warning. Never let a monitoring failure cascade into an execution halt.
- **Checking all limits after halt:** Once `halt_flag = true`, skip all three limit checks and return `HaltAll` immediately. Avoid running IL or Drift checks after the process is already halted.
- **Modifying `peak_pnl` on loss:** `peak_pnl` is a high-water mark. It only ever increases. Update it when `snap.net_pnl > state.peak_pnl`. Never update it when net_pnl drops.
- **Drawdown formula with zero peak:** Guard against `peak_pnl = 0.0` (freshly initialized state). Division by zero if peak is zero. Use: if `peak_pnl <= 0.0`, skip drawdown check (no peak established yet).

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| DB upsert idempotency | Custom conflict handling | `ON CONFLICT DO UPDATE` (PostgreSQL) | Already used in `pool_ticks`; standard SQL |
| Async fire-and-forget | Mutex-guarded write queue | `tokio::spawn` | Already established pattern in `spawn_pnl_write` |
| Drift account borsh skip | Custom byte scanner | Skip first 8 bytes (`&data[8..]`), then `borsh::BorshDeserialize::deserialize` | Anchor discriminator is always 8 bytes; borsh 0.10 already in Cargo.toml |
| Drawdown peak tracking | Rolling window average | Simple high-water mark (max ever seen) | Drawdown = cumulative from peak by definition |

---

## Common Pitfalls

### Pitfall 1: Race Between Async DB Write and Next-Tick State Read

**What goes wrong:** `spawn_pnl_write` writes the snapshot fire-and-forget. The risk monitor reads the same data from the in-memory `PnlSnapshot` passed to `evaluate()`, NOT from a SELECT against the DB. If the risk monitor tries to SELECT the latest `pnl_history` row to get `il_usd`, it may read a row from the previous tick (the current tick's write is still in-flight).

**Why it happens:** Fire-and-forget writes are non-blocking by design (PERSIST-03).

**How to avoid:** Pass the `PnlSnapshot` struct directly to `evaluate()` as its input. The risk monitor does NOT query `pnl_history`. It uses the already-computed `snap.il_usd`, `snap.net_pnl`, and `snap.position_value` from the current tick's values.

**Warning signs:** Risk monitor SQL SELECT code; risk monitor taking a `PgPool` as its primary data source.

### Pitfall 2: halt_flag Cleared On Restart

**What goes wrong:** Developer adds a "fresh start" init path that creates a new `risk_state` row unconditionally, clearing `halt_flag`.

**Why it happens:** The init logic for "no row exists" is similar to "reset state."

**How to avoid:** `load_or_init` must be: SELECT the row; if no row exists, INSERT fresh row with `halt_flag = false`. If the row exists — including with `halt_flag = true` — return it as-is. Never update `halt_flag` to false on startup. Log a CRITICAL warning if `halt_flag = true` is found at startup.

**Warning signs:** `INSERT ... ON CONFLICT DO UPDATE SET halt_flag = false` or similar.

### Pitfall 3: Drawdown Pct Computed Before peak_pnl Is Established

**What goes wrong:** `peak_pnl = 0` (initial state); `net_pnl = -5.0`; drawdown formula fires with division by zero or yields `(0 - (-5)) / 0 = inf`, triggering a spurious halt on the first ever tick.

**How to avoid:** Guard: `if self.state.peak_pnl <= 0.0 { skip drawdown check }`. Only enable drawdown check once a positive `peak_pnl` has been recorded.

### Pitfall 4: IL Percentage Computed With Signed il_usd

**What goes wrong:** `il_usd` is always negative (IL is a loss). `il_pct = il_usd / position_value` yields a negative percentage. Comparison `il_pct > max_il_pct` (where max_il is positive) always false — IL check never fires.

**How to avoid:** `il_pct = il_usd.abs() / position_value`. The CONTEXT.md specifics explicitly state: `IL percentage = |il_usd| / position_value`.

### Pitfall 5: Drift User PDA Not Derivable Without a Configured Wallet

**What goes wrong:** In shadow mode with no `WALLET_KEYPAIR`, the Drift User PDA cannot be derived (no authority pubkey). Drift margin check panics or errors.

**How to avoid:** If `--drift-min-margin-ratio` is set but `wallet_keypair` is `None` (shadow mode), skip the Drift check and log a warning: "drift margin check skipped (no keypair in shadow mode)". The `drift_user_pubkey: Option<Pubkey>` on `RiskMonitor` handles this cleanly — if `None`, skip the check.

### Pitfall 6: sqlx-core execute() vs fetch_one() for Upsert

**What goes wrong:** Using `fetch_one()` for an upsert that returns no rows. Panics with "no rows returned."

**How to avoid:** Use `pool.execute(query(...))` for INSERT/UPDATE. Use `fetch_one()` / `fetch_optional()` only for SELECT. This matches the established pattern in `write_pool_tick` and `write_pnl_snapshot`.

---

## Code Examples

### risk_state DB Table (schema.sql addition)

```sql
-- Risk state persistence (RISK-04): one row per pool_address, upserted on every tick.
-- halt_flag survives restart — operator must manually clear via SQL (D-12).
CREATE TABLE IF NOT EXISTS risk_state (
    pool_address          TEXT PRIMARY KEY,
    peak_pnl              DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    current_drawdown_pct  DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    pause_flag            BOOLEAN NOT NULL DEFAULT FALSE,
    halt_flag             BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

Note: `PRIMARY KEY` on `pool_address` provides the ON CONFLICT target. No BIGSERIAL needed — this is a single-row-per-pool state record, not an append-only log. [VERIFIED: consistent with shadow_rebalances which also uses pool_address as key for lookups]

### CLI Flag Addition (Commands::Watch)

```rust
// Source: derived from existing --max-slippage-bps pattern [VERIFIED: codebase]
Watch {
    mint: String,
    #[arg(long, conflicts_with = "live")]
    shadow: bool,
    #[arg(long, conflicts_with = "shadow")]
    live: bool,
    #[arg(long, default_value_t = 50)]
    max_slippage_bps: u32,
    // Phase 6 additions:
    /// Maximum cumulative P&L drawdown as percentage (e.g. 10.0 = 10%).
    /// When exceeded, LP position is closed and rebalancing halts permanently.
    #[arg(long)]
    max_drawdown: Option<f64>,
    /// Maximum instantaneous IL as percentage of position value (e.g. 5.0 = 5%).
    /// When exceeded, rebalancing is paused until IL recovers.
    #[arg(long)]
    max_il: Option<f64>,
    /// Minimum Drift margin ratio as percentage (e.g. 20.0 = 20%).
    /// When below this, Drift hedge close is logged (CPI deferred to LIVE-02).
    #[arg(long)]
    drift_min_margin_ratio: Option<f64>,
}
```

Using `Option<f64>` means the limit is disabled when the flag is not passed — no validation penalty for users who don't configure all three limits.

### Drawdown Calculation

```rust
// Source: CONTEXT.md §Specific Implementation Notes [VERIFIED: codebase]
// peak_pnl is high-water mark; only updated when snap.net_pnl > state.peak_pnl
if snap.net_pnl > self.state.peak_pnl {
    self.state.peak_pnl = snap.net_pnl;
}

// Drawdown check: guard against zero/negative peak (no established peak yet)
if let Some(max_dd) = self.max_drawdown_pct {
    if self.state.peak_pnl > 0.0 {
        let drawdown_pct = (self.state.peak_pnl - snap.net_pnl) / self.state.peak_pnl * 100.0;
        self.state.current_drawdown_pct = drawdown_pct;
        if drawdown_pct > max_dd {
            // trigger close_all action ...
        }
    }
}
```

### IL Calculation

```rust
// Source: CONTEXT.md §Specific Implementation Notes
// il_usd is negative (loss); use abs() for percentage comparison
let il_pct = if snap.position_value > 0.0 {
    snap.il_usd.abs() / snap.position_value * 100.0
} else {
    0.0
};

match self.max_il_pct {
    Some(max_il) if il_pct > max_il && !self.state.pause_flag => {
        self.state.pause_flag = true;
        return Ok(RiskAction::PauseRebalancing { il_pct });
    }
    Some(max_il) if il_pct <= max_il && self.state.pause_flag => {
        self.state.pause_flag = false;
        return Ok(RiskAction::ResumeRebalancing { il_pct });
    }
    Some(_) if self.state.pause_flag => {
        // Still paused, still above threshold — propagate pause
        return Ok(RiskAction::PauseRebalancing { il_pct });
    }
    _ => {}
}
```

### Drift User Account Fetch + Proxy Margin Ratio

```rust
// Source: [ASSUMED based on borsh 0.10 and solana-client 1.18 APIs in Cargo.toml]
// The Drift User account layout: [8-byte discriminator][borsh-encoded User struct]
// We only deserialize the fields needed for a proxy metric.

#[derive(borsh::BorshDeserialize)]
struct DriftPerpPosition {
    base_asset_amount: i64,        // signed, in base asset units (1e-9 precision)
    quote_asset_amount: i64,       // signed USDC equivalent in 1e-6 precision
    // ... other fields we skip via dummy padding
    _padding: [u8; 88],            // remaining PerpPosition fields (count TBD by IDL)
}

// Fetch
let rpc = solana_client::rpc_client::RpcClient::new_with_timeout(
    rpc_url.to_string(),
    std::time::Duration::from_secs(rpc_timeout),
);
match rpc.get_account_data(&drift_user_pubkey) {
    Ok(data) if data.len() > 8 => {
        let body = &data[8..]; // skip 8-byte Anchor discriminator
        // Deserialize partial User struct...
    }
    Ok(_) => { tracing::warn!("drift user account too short — skipping margin check"); }
    Err(e) => { tracing::warn!(error = %e, "drift user RPC fetch failed — margin check skipped"); }
}
```

**Important:** The exact borsh field offsets for `DriftPerpPosition` must be verified against the official Drift protocol-v2 IDL JSON at implementation time. The `_padding` size above is approximate — do NOT use it literally without confirming against the actual struct layout. [ASSUMED field layout]

### main.rs Integration Point

```rust
// After spawn_pnl_write, before should_rebalance() — per D-05 evaluation order
// [VERIFIED: main.rs line ~1014 is spawn_pnl_write; rebalance decision is at line ~679]

// Risk gate
let risk_action = risk_monitor.evaluate(&snap).await?;
match risk_action {
    RiskAction::HaltAll { drawdown_pct } => {
        tracing::error!(drawdown_pct, "risk: halt_flag active — skipping rebalance");
        return; // exit tick callback, skip slippage + rebalance
    }
    RiskAction::PauseRebalancing { il_pct } => {
        tracing::warn!(il_pct, "risk: IL pause active — skipping rebalance");
        return;
    }
    RiskAction::ResumeRebalancing { il_pct } => {
        tracing::info!(il_pct, "risk: IL recovered — resuming rebalance");
        // fall through to should_rebalance()
    }
    RiskAction::CloseDriftHedge { margin_ratio } => {
        tracing::error!(
            margin_ratio,
            "risk: Drift margin below threshold — hedge close deferred (LIVE-02)"
        );
        // LP rebalance continues (RISK-03: only Drift hedge affected)
    }
    RiskAction::Continue => {
        // fall through to should_rebalance()
    }
}
```

Note: `CloseDriftHedge` does NOT halt the LP rebalance path. RISK-03 only affects the Drift side. The tick continues to `should_rebalance()` after the CRITICAL log.

---

## State of the Art

| Old Approach | Current Approach | Impact |
|--------------|------------------|--------|
| Hard-coded kill switch (process exit on breach) | `halt_flag` in DB with process continuation | Operator retains visibility; no lost reconnect on restart |
| Check risk only on rebalance trigger | Check on every tick (D-04) | Faster breach detection; IL oscillation caught promptly |
| Risk state in memory only | Risk state persisted to DB (RISK-04) | Survives restarts; operator can query state via SQL |

---

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | Drift User account has an 8-byte Anchor discriminator prefix before the borsh payload | Code Examples — Drift borsh fetch | Deserialization fails at startup; falls into warning/skip path (benign but RISK-03 non-functional) |
| A2 | Drift User PDA = `find_program_address(["user", authority, [0u8, 0u8]], drift_program_id)` with `dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH` | Architecture Pattern 7 | Wrong PDA → account not found → margin check silently skipped (per fallback rule) |
| A3 | Borsh field layout of `PerpPosition` in Drift protocol-v2 — struct size and field offsets | Code Examples — DriftPerpPosition | Partial deserialization corrupts subsequent fields; proxy metric wrong. Must verify against Drift IDL JSON at implementation time |
| A4 | `drift-rs` v1.0.0 requires `libdrift` native library (assessment based on GitHub README warning) | Standard Stack — Why Not Use drift-rs | If wrong, drift-rs may be usable; revisit crate choice during implementation |

**Assumed claims that need verification at implementation time:**
- A1, A2, A3: verify against `https://github.com/drift-labs/protocol-v2` IDL JSON and on-chain account inspection before coding the Drift deserialization path.

---

## Open Questions

1. **Drift PerpPosition borsh layout field count and padding**
   - What we know: User struct has `perp_positions: [PerpPosition; 8]` and `spot_positions: [SpotPosition; 8]`
   - What's unclear: Exact byte size of each `PerpPosition` struct in the Drift v2 protocol-v2 borsh layout (varies with anchor version and struct additions)
   - Recommendation: At implementation time, read the IDL JSON from `https://github.com/drift-labs/protocol-v2/blob/master/sdk/src/idl/drift.json` and count fields. Alternatively, use `solana-account-decoder` to fetch and inspect a live Drift User account on devnet to confirm the byte layout empirically.

2. **Risk monitor in the tick closure vs. as a separate struct**
   - What we know: The watch loop currently runs in a `Box<dyn Fn(serde_json::Value) + Send>` closure (non-async). `risk_monitor.evaluate()` needs to be async (Drift RPC call).
   - What's unclear: The existing tick callback is synchronous (no `.await` inside). Adding an async call requires either making the closure async (requires `tokio::task::block_in_place` or restructuring) or spawning the Drift check separately.
   - Recommendation: Use `tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(...))` for the Drift RPC call inside the sync closure, OR restructure the Drift fetch as a `tokio::spawn` that caches the last known margin ratio (fetched asynchronously, read synchronously). The latter is cleaner and avoids blocking the event loop.

3. **`RiskAction::CloseDriftHedge` interaction with rebalance path**
   - What we know: RISK-03 only affects Drift; LP rebalance continues (per CONTEXT.md).
   - What's unclear: If RISK-03 fires on the same tick as a rebalance decision, should both the CRITICAL log AND the rebalance execute?
   - Recommendation: Yes — the tick continues to `should_rebalance()` after the `CloseDriftHedge` action is handled. The match arm should NOT return early. This is the correct behavior per CONTEXT.md ("LP stays open").

---

## Environment Availability

Step 2.6: SKIPPED — Phase 6 is a code/config-only change. No new external tools, services, or CLIs required. All external dependencies (PostgreSQL, Solana RPC endpoint) were already required and verified in prior phases.

---

## Project Constraints (from CLAUDE.md)

| Directive | Source | Applies To Phase 6 |
|-----------|--------|---------------------|
| Use `anyhow` for all error handling; no `unwrap()` in production paths | CLAUDE.md | `RiskMonitor::evaluate()`, all DB and RPC calls |
| Keypairs only via environment variables | CLAUDE.md | Drift User PDA derived from wallet keypair already loaded via `WALLET_KEYPAIR` env var |
| Math must be validated via proptest property-based tests where applicable | CLAUDE.md | Risk threshold calculations (drawdown pct, IL pct) are pure functions → unit tests required |
| No external deps unless necessary | CLAUDE.md (implied) | No new crates; borsh 0.10 and solana-client 1.18 already present |
| `cargo clippy -- -D warnings` must pass | CLAUDE.md | New module must compile cleanly |
| `cargo fmt` required | CLAUDE.md | Format new files before commit |

---

## Sources

### Primary (HIGH confidence)
- Codebase grep of `src/main.rs`, `src/strategy/`, `src/execution/`, `src/storage/` — direct code inspection for integration points, existing patterns, and method signatures [VERIFIED]
- `Cargo.toml` — confirmed dependency versions and existing crates [VERIFIED]
- `src/storage/schema.sql` — confirmed existing table patterns and UNIQUE constraint idioms [VERIFIED]
- `.planning/phases/06-risk-limits/06-CONTEXT.md` — locked decisions, discretion areas, implementation specifics [VERIFIED]

### Secondary (MEDIUM confidence)
- `https://github.com/drift-labs/drift-rs` — confirmed v1.0.0 release March 2026, gRPC/yellowstone-grpc deps, libdrift requirement [CITED]
- `https://docs.rs/drift-rs/latest/drift_rs/` — confirmed major dependency list [CITED]
- `https://docs.drift.trade/trading/account-health` — confirmed margin ratio formula and components [CITED]
- `https://github.com/drift-labs/protocol-v2/blob/master/programs/drift/src/state/user.rs` — confirmed User struct fields; margin ratio requires on-chain oracle context (not stored in User account) [CITED]
- `https://github.com/drift-labs/gateway` — confirmed `/v2/user/marginInfo` REST endpoint (alternative to raw borsh deserialize if gateway is running) [CITED]

### Tertiary (LOW confidence)
- Drift program ID `dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH` — known from training data; should be verified before deployment [ASSUMED]
- Drift User PDA derivation seeds `["user", authority, subaccount_index]` — known from training data and protocol-v2 repo patterns [ASSUMED]
- `PerpPosition` borsh struct size — not directly verified in this research session; must be checked against IDL JSON at implementation [ASSUMED]

---

## Metadata

**Confidence breakdown:**
- Standard Stack: HIGH — all crates verified in Cargo.toml; drift-rs rejection confirmed via GitHub/docs.rs
- Architecture: HIGH — integration points verified in codebase; patterns derived from CONTEXT.md locked decisions
- Pitfalls: HIGH — derived from codebase patterns and logic analysis of the state machine design
- Drift deserialization: LOW-MEDIUM — the approach is sound but exact field offsets are ASSUMED; must be verified at implementation time

**Research date:** 2026-04-10
**Valid until:** 2026-05-10 (Drift IDL evolves; verify PerpPosition layout before implementation)
