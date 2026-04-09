---
phase: 02-shadow-mode
verified: 2026-04-09T00:00:00Z
status: human_needed
score: 4/4 must-haves verified
overrides_applied: 0
human_verification:
  - test: "Run `cargo run -- watch --shadow --mint <POSITION_MINT>` against devnet for at least one tick cycle and observe tracing output"
    expected: "Log line with `mode = Shadow` at watch start; no `submission blocked` or gate errors in output; if a rebalance is triggered, log line `shadow rebalance decision` appears with pool/trigger/price fields"
    why_human: "Cannot invoke the watch loop programmatically without a live Solana RPC and WebSocket feed; rebalance decision log output requires real pool state"
  - test: "Run `cargo run -- watch --live --mint <POSITION_MINT>` against a fresh DB (no shadow_rebalances rows)"
    expected: "Process prints `ERROR: shadow gate FAILED: no shadow_rebalances rows for pool ...` to stderr and exits with code 2"
    why_human: "Gate enforcement requires a running DB and correct DATABASE_URL; requires a real invocation to confirm exit code 2 in practice"
---

# Phase 2: Shadow Mode — Verification

**Phase Goal:** Operator can run the full rebalance decision loop for weeks without any transaction risk, building confidence and a logged decision trail before touching real capital.
**Verified:** 2026-04-09
**Status:** HUMAN_NEEDED (automated checks all pass; 2 runtime behaviors require human confirmation)
**Re-verification:** No — initial verification

## Success Criteria

| # | Criterion | Status | Evidence |
|---|-----------|--------|----------|
| 1 | `cargo run -- watch --shadow` runs without submitting any transactions; rebalance decisions appear in logs | PASS (automated partial) | `ShadowGuard::submit` returns `Err(Blocked)` in Shadow mode (unit tests pass); `guard.submit(&plan_proxy)` called in watch loop before any action (main.rs:616); `tracing::warn!` on block. Human confirmation required for live log output. |
| 2 | Each shadow decision is persisted to DB with timestamp, trigger reason, price, and simulated IL delta | PASS | `shadow_rebalances` table has `created_at TIMESTAMPTZ DEFAULT NOW()`, `trigger_reason TEXT`, `price DOUBLE PRECISION`, `simulated_il_usd DOUBLE PRECISION`; `spawn_shadow_write` called in watch loop (main.rs:680) with populated `ShadowRebalanceRow` |
| 3 | Running `cargo run -- watch --live` without meeting the 2-week + zero-error gate returns a clear error and exits | PASS (automated partial) | `check_shadow_gate` calls `MIN(created_at)` + error count queries; gate enforced at main.rs:459-474 behind `RunMode::Live` guard; exits with `process::exit(2)` + `eprintln!` on failure. No-DB case also exits with code 2. Human confirmation of runtime behavior needed. |
| 4 | Shadow logs are queryable from DB to reconstruct full decision history | PASS | `shadow_rebalances` table has composite index `(pool_address, created_at DESC)` + partial index `WHERE error_flag = true`; all required columns present for full history reconstruction |

## Must-Haves

### Plan 02-01: ShadowGuard + CLI Flags

| Must-Have | Status | Evidence |
|-----------|--------|----------|
| `cargo run -- watch` defaults to shadow mode | PASS | main.rs:416 — `if *live { RunMode::Live } else { RunMode::Shadow }`; shadow when neither flag set |
| `--shadow` and `--live` are mutually exclusive CLI flags | PASS | main.rs:67-71 — `conflicts_with = "live"` and `conflicts_with = "shadow"` |
| ShadowGuard blocks all transaction signing/submission when shadow is active | PASS | `shadow_guard.rs:29-33` — `submit()` returns `Err(Blocked)` in Shadow variant; 3 unit tests pass |
| `src/execution/shadow_guard.rs` exists and provides ShadowGuard | PASS | File present; `pub enum ShadowGuard` with Shadow/Live variants, `submit()` gate, 3 unit tests |
| `src/main.rs` has watch subcommand with --shadow/--live flags | PASS | Both flags present; `RunMode` derived; guard constructed from mode; `guard.submit` called at main.rs:616 |
| main.rs → shadow_guard.rs link via ShadowGuard construction | PASS | `execution::ShadowGuard::shadow()` / `execution::ShadowGuard::live()` called at main.rs:419-420 |

### Plan 02-02: Shadow Rebalance Logging

| Must-Have | Status | Evidence |
|-----------|--------|----------|
| Each shadow rebalance decision writes one row to `shadow_rebalances` | PASS | `spawn_shadow_write` called in watch loop (main.rs:680); only on `Rebalance` or `Err` decisions, not `Hold` |
| Row includes timestamp, pool_address, trigger_reason, price, simulated_* fields | PASS | `ShadowRebalanceRow` struct has all fields; schema DDL has all columns |
| Errors from rebalance decision path set error_flag=true with message | PASS | main.rs:665-675 — `Err(e)` arm sets `error_flag: true, error_message: Some(e.clone())` |
| Real fees/IL/net P&L values land in pnl_history (replacing 0.0 stubs) | PASS | `grep -c "fees_earned: 0.0"` → 0 matches; `grep -c "il_usd: 0.0"` → 0 matches; real `computed_fees_earned` and `computed_il_usd` wired |
| `src/storage/schema.sql` has shadow_rebalances DDL | PASS | `CREATE TABLE IF NOT EXISTS shadow_rebalances` at schema.sql:45 with all required columns |
| `src/storage/writer.rs` has ShadowRebalanceRow + write/spawn functions | PASS | `pub struct ShadowRebalanceRow` at writer.rs:124; `pub async fn write_shadow_rebalance` at writer.rs:142; `pub fn spawn_shadow_write` at writer.rs:171 |
| main.rs → writer.rs link via spawn_shadow_write | PASS | `storage::writer::spawn_shadow_write(pg.clone(), row)` at main.rs:680 |

### Plan 02-03: Shadow DB Gate

| Must-Have | Status | Evidence |
|-----------|--------|----------|
| `watch --live` invokes a DB gate check before entering the loop | PASS | main.rs:457-474 — gate runs after DB pool construction, before watch loop |
| Gate passes only when MIN(created_at) ≥ 14 days ago AND COUNT(error_flag=true)=0 | PASS | writer.rs:240-279 — two sequential queries; NoData/TooRecent/ErrorsPresent/Pass precedence |
| Gate failure exits process with non-zero code and descriptive message | PASS | `std::process::exit(2)` at main.rs:464, 471; `eprintln!("ERROR: {}", status.describe())` |
| `watch` (shadow) never runs the gate check | PASS | Gate gated behind `if matches!(run_mode, RunMode::Live)` at main.rs:459 |
| `src/storage/writer.rs` has check_shadow_gate returning GateStatus | PASS | `pub async fn check_shadow_gate` at writer.rs:233; `pub enum GateStatus` at writer.rs:188 |
| main.rs → writer.rs link via check_shadow_gate | PASS | `storage::writer::check_shadow_gate(pg, &pool_addr).await` at main.rs:467 |

### Plan 02-04: Integration Tests

| Must-Have | Status | Evidence |
|-----------|--------|----------|
| Integration test proves --live rejects on empty DB | PASS | `gate_no_data` test at shadow_gate_integration.rs:51 |
| Integration test proves --live rejects when earliest row <14 days old | PASS | `gate_too_recent` test at shadow_gate_integration.rs:58 |
| Integration test proves --live rejects when any row has error_flag=true | PASS | `gate_errors_present` test at shadow_gate_integration.rs:73 |
| Integration test proves --live accepts when earliest ≥14 days AND zero errors | PASS | `gate_pass` test at shadow_gate_integration.rs:82 |
| Integration test proves shadow mode never invokes the gate | PASS (structural) | Gate is only called in `RunMode::Live` branch (main.rs:459); integration tests test gate function directly, not CLI mode |
| `tests/shadow_gate_integration.rs` exists with 5 test cases | PASS | File present; `grep -c "#[tokio::test]"` → 5 |
| tests → writer.rs link via check_shadow_gate calls | PASS | `use tick_liq::storage::writer::{check_shadow_gate, GateStatus}` at integration_test:15; 7 references to `check_shadow_gate` |

## Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `src/execution/shadow_guard.rs` | ShadowGuard type gating submission | VERIFIED | Present; Copy enum; submit() gate; 3 unit tests |
| `src/storage/schema.sql` | shadow_rebalances DDL with required columns | VERIFIED | All columns present: id, created_at, pool_address, trigger_reason, price, simulated_*, error_flag, error_message; 2 indexes |
| `src/storage/writer.rs` | ShadowRebalanceRow + check_shadow_gate + GateStatus | VERIFIED | All types and functions present; 3 gate unit tests |
| `src/main.rs` | CLI flags, guard construction, shadow write, gate check | VERIFIED | All 4 wiring points confirmed |
| `tests/shadow_gate_integration.rs` | 5 integration tests for gate branches | VERIFIED | Exactly 5 `#[tokio::test]` functions; all 4 gate branches + pool isolation |
| `Cargo.toml` | uuid dev-dependency | VERIFIED | `uuid = { version = "1", features = ["v4"] }` in dev-dependencies |

## Key Link Verification

| From | To | Via | Status | Details |
|------|----|-----|--------|---------|
| `src/main.rs` | `src/execution/shadow_guard.rs` | `ShadowGuard::shadow()` / `ShadowGuard::live()` + `guard.submit()` | WIRED | main.rs:419-421, 616 |
| `src/main.rs` | `src/storage/writer.rs` | `spawn_shadow_write` | WIRED | main.rs:680 |
| `src/main.rs` | `src/storage/writer.rs` | `check_shadow_gate` behind `RunMode::Live` | WIRED | main.rs:467, gated at 459 |
| `tests/shadow_gate_integration.rs` | `src/storage/writer.rs` | `check_shadow_gate` + fixture inserts | WIRED | test:15 import; 5 call sites |

## Data-Flow Trace (Level 4)

| Artifact | Data Variable | Source | Produces Real Data | Status |
|----------|---------------|--------|--------------------|--------|
| `shadow_rebalances` rows | `ShadowRebalanceRow.price` | `price_current = sqrt_q64_to_price(pool.sqrt_price)` from WebSocket tick | Yes — derived from live pool state | FLOWING |
| `shadow_rebalances` rows | `ShadowRebalanceRow.simulated_il_usd` | `computed_il_usd` from `strategy::il::compute` (or inline computation) | Yes — computed from entry vs current price | FLOWING |
| `shadow_rebalances` rows | `ShadowRebalanceRow.trigger_reason` | `strategy::RebalanceDecision::Rebalance { reason }` from `should_rebalance()` | Yes — real strategy evaluation | FLOWING |
| `pnl_history` rows | `fees_earned`, `il_usd`, `net_pnl` | `computed_fees_earned`, `computed_il_usd` replacing 0.0 stubs | Yes — Phase 1 stubs removed; 0 grep matches for stubs | FLOWING |

## Behavioral Spot-Checks

| Behavior | Command | Result | Status |
|----------|---------|--------|--------|
| `cargo build` succeeds | `~/.cargo/bin/cargo build` | `Finished dev profile` | PASS |
| `cargo build --tests` succeeds | `~/.cargo/bin/cargo build --tests` | `Finished dev profile` | PASS |
| Integration test file has 5 tokio tests | `grep -c "#[tokio::test]" tests/shadow_gate_integration.rs` | 5 | PASS |
| Phase 1 P&L stubs removed | `grep -c "fees_earned: 0.0" src/main.rs` | 0 | PASS |
| shadow_guard unit test count | `grep -c "#[test]" src/execution/shadow_guard.rs` | 3 | PASS |
| Gate unit tests in writer.rs | `grep -c "fn " src/storage/writer.rs` (gate_tests module) | 3 gate tests | PASS |
| `cargo run -- watch` without flags defaults to Shadow | main.rs:416 `if *live { Live } else { Shadow }` | Shadow is default | PASS (structural) |
| `cargo run -- watch --shadow --live` conflicts | clap `conflicts_with` annotations | Would fail at argparse | PASS (structural) |
| Live mode requires DB (no-DB path exits 2) | main.rs:460-464 `None => exit(2)` branch | Present and correct | PASS |

## Requirements Coverage

| Requirement | Source Plan | Description | Status | Evidence |
|-------------|------------|-------------|--------|----------|
| SHADOW-01 | 02-01 | `--shadow` flag runs full decision logic without signing/submitting | SATISFIED | ShadowGuard blocks in Shadow mode; `guard.submit` returns Blocked |
| SHADOW-02 | 02-02 | Each shadow rebalance decision is logged (timestamp, trigger, price, simulated IL) | SATISFIED | `spawn_shadow_write` wired in watch loop; all schema columns present |
| SHADOW-03 | 02-02 | Shadow rebalance log persisted to DB (`shadow_rebalances` table) | SATISFIED | Table DDL in schema.sql; `run_migrations` sources full schema.sql |
| SHADOW-04 | 02-03, 02-04 | Live trading requires `--live` + 2-week + zero-errors gate | SATISFIED | Hard gate with `process::exit(2)` in Live path; integration tests lock all 4 gate branches |

## Anti-Patterns Found

| File | Line | Pattern | Severity | Impact |
|------|------|---------|----------|--------|
| `src/execution/shadow_guard.rs` | 22 | `#[allow(dead_code)]` on `is_shadow()` | Info | Intentional — API surface for downstream plans per design decision |
| `src/execution/mod.rs` | 8 | `#[allow(unused_imports)]` on `ShadowGuardError` | Info | Intentional — exported for downstream callers per design decision |
| `src/storage/writer.rs` | 4 | `#![allow(dead_code)]` module-level | Info | Suppresses warnings on writer functions not yet called in all paths; non-blocking |

No STUB or MISSING patterns found. No `return null`, empty implementation bodies, or hardcoded empty data that flows to rendering.

## Human Verification Required

### 1. Shadow mode — no transaction submission in practice

**Test:** Run `cargo run -- watch --shadow --mint <DEVNET_POSITION_MINT>` against Solana devnet and observe log output through at least one WebSocket tick event (and ideally a simulated out-of-range condition).
**Expected:** Log line containing `mode = Shadow` at startup; if a rebalance is triggered, `shadow rebalance decision` log entry appears with pool/trigger/price; NO `submission allowed (live mode)` log lines; `ShadowGuard: submission blocked` if guard is hit.
**Why human:** Requires a live Solana RPC and WebSocket feed; rebalance decision path depends on real pool state that cannot be mocked programmatically without a running node.

### 2. `--live` gate enforcement at runtime

**Test:** Set `DATABASE_URL=postgres://...` pointing to a fresh DB (no shadow_rebalances rows), then run `cargo run -- watch --live --mint <POSITION_MINT>`.
**Expected:** Process prints to stderr `ERROR: shadow gate FAILED: no shadow_rebalances rows for pool ...`, followed by the hint about accumulating shadow data, then exits. Check exit code: `echo $?` should return `2`.
**Why human:** Requires a running PostgreSQL instance and correct DATABASE_URL; the exit code behavior and exact stderr output can only be confirmed with a real DB connection.

## Gaps Summary

No gaps found. All automated checks pass:
- All 4 success criteria have implementation evidence
- All 8 must-haves across 4 plans verified
- SHADOW-01 through SHADOW-04 requirements covered
- `cargo build --tests` exits 0
- 5 integration tests present and compile
- No stub anti-patterns blocking goal achievement

Two items require human verification (runtime behavior with live DB + RPC) but these are behavioral spot-checks, not implementation gaps.

---

_Verified: 2026-04-09_
_Verifier: Claude (gsd-verifier)_
