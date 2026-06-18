---
phase: 06-risk-limits
review_date: 2026-04-10
reviewer: Senior Code Reviewer (Claude Sonnet 4.6)
build_status: PASS
test_status: 18 passed, 1 ignored, 0 failed
clippy_status: PASS (0 project warnings)
fmt_status: PASS
verdict: APPROVE with noted items
---

# Phase 06: Risk Limits — Code Review

## Overview

All three plans (01 core module, 02 DB+RPC, 03 watch-loop wiring) have been executed. The build passes, all 18 unit tests pass, clippy is clean, and formatting is correct. All plan must_haves are satisfied. The review identifies two important issues, four suggestions, and one plan deviation that was well-handled.

---

## What Was Done Well

**Correct evaluation order and halt semantics.** The `evaluate()` evaluation order (halt_flag gate → high-water mark → drawdown → IL → Drift margin → Continue) exactly matches D-05/D-06. The `halt_flag` is set synchronously in `evaluate()` and persisted immediately afterward, meaning there is no window between in-memory breach detection and durable storage of the halt.

**SELECT-then-INSERT for halt_flag preservation.** The `load_or_init()` implementation correctly uses a SELECT followed by a conditional INSERT rather than an upsert, which is the only safe pattern for D-12 (halt_flag must survive restart). The `ON CONFLICT DO NOTHING` guard handles the race without ever overwriting an existing halt_flag=true row.

**Fire-and-forget persist matches established pattern.** `persist_state()` exactly mirrors `spawn_pnl_write` from `storage::writer` — synchronous entry point, tokio::spawn internally, warn! on failure. This is the right pattern for non-blocking tick processing.

**Test coverage is thorough.** All 13 behaviors from the Plan 01 task spec are covered by named tests, including the tricky edge cases (peak_pnl <= 0 guard, position_value=0 guard, IL oscillation with pause_flag already set, evaluation order priority). The test helper functions (`make_state`, `make_snap`, `monitor_all`) keep test bodies readable.

**RPC failure degrades gracefully.** `fetch_drift_margin_ratio()` returns `None` on every error path (account not found, data too short, network timeout) with structured `warn!` logs rather than propagating errors that could halt tick processing. The 5-second `RpcClient` timeout aligns with T-06-05.

**Arc<Mutex<RiskMonitor>> justification is documented.** The summary correctly explains why `Arc<Mutex<T>>` was required: the WebSocket `NotifyFn` callback is `Fn`, not `FnMut`, so interior mutability is necessary. This is a non-obvious Rust constraint and the decision is sound.

**The max_slippage_bps removal is correct.** The plan spec showed `max_slippage_bps` still present in the Watch struct interface document, but it had already been removed in a prior phase. The implementation correctly omits it without introducing a regression.

---

## Issues

### Important (Should Fix)

**I-01: `lock().unwrap()` on `Arc<Mutex<RiskMonitor>>` in the tick callback is not panic-safe**

File: `src/main.rs`, lines 765, 775, 794, 808, 822, 835, 845

The tick callback acquires the mutex seven times per tick using `.lock().unwrap()`. If any future code path panics while holding the lock, the mutex becomes poisoned, and all subsequent ticks will panic at `.unwrap()`, crashing the process. This is particularly relevant here because the callback runs inside a long-lived WebSocket loop that is expected to be resilient.

The project convention (`CLAUDE.md`: "no `unwrap()` in production paths") is also directly violated.

The fix is to replace each `.lock().unwrap()` with `.lock().unwrap_or_else(|p| p.into_inner())` (recover from poison) or more idiomatically `.lock().expect("risk_monitor mutex poisoned")` — but the correct production-quality fix is to use `match risk_arc.lock() { Ok(g) => g, Err(e) => { tracing::error!("risk_monitor mutex poisoned: {}", e); return; } }`. Either the recover-from-poison pattern or the explicit error-with-return pattern should be used consistently.

**I-02: The Drift User account byte layout relies on magic offsets with no verification**

File: `src/strategy/risk_monitor.rs`, lines 296–336

The constants `PERP_ARRAY_OFFSET = 4400` and `PERP_POSITION_SIZE = 136` are hardcoded approximations of the Drift v2 User account layout. The code comment says "Layout approximation" and "RESEARCH.md Pattern 6" but the RESEARCH.md itself describes this as an open question. The Drift User account is 4,216 bytes in v2 (`User` struct size per on-chain program), so an offset of 4,400 bytes will exceed `payload.len()` for most real accounts, causing the inner `if payload.len() >= PERP_ARRAY_OFFSET + ...` guard to always evaluate false. When the guard is false, `total_base_abs` and `total_quote_abs` both remain 0, and the proxy ratio computes to `0.0 / 1.0 = 0.0`. A ratio of 0.0 is always below any configured `drift_min_margin_ratio` threshold, meaning the margin check would fire on every tick with a live Drift account — the opposite of the intended "fail open" (D-03: treat RPC failure as "margin OK") behavior.

The code comment marking this as an approximation and deferring full calculation to LIVE-02 is correct policy. However, the current fallback when the offset guard fails is to return `Some(0.0)` rather than `None`, which would incorrectly trigger `CloseDriftHedge` on every tick rather than skipping the check. This is a latent correctness bug that will activate the moment `drift_user_pubkey` and `drift_min_margin_ratio` are both set.

The immediate fix without changing scope is to return `None` (skip check) when the payload is too short to contain the expected structure, rather than computing a ratio from zeroed accumulators. Add an explicit check after the offset guard and log a warning:

```rust
// If the payload is shorter than expected, return None (skip check, don't trigger false breach).
if payload.len() < PERP_ARRAY_OFFSET + MAX_PERP_POSITIONS * PERP_POSITION_SIZE {
    warn!(len = payload.len(), expected = PERP_ARRAY_OFFSET + MAX_PERP_POSITIONS * PERP_POSITION_SIZE,
        "drift user account shorter than expected layout -- margin check skipped");
    return None;
}
```

This converts the current behavior (silently compute 0.0 ratio → always trigger CloseDriftHedge) into the intended behavior (skip check, return None → Continue).

---

### Suggestions (Nice to Have)

**S-01: Redundant branching in the IL check can be collapsed**

File: `src/strategy/risk_monitor.rs`, lines 387–396

The two branches `if !self.state.pause_flag` and `else` (already paused) both set `updated_at` and return `PauseRebalancing { il_pct }`. The `pause_flag = true` assignment only fires on the first branch but it is idempotent (setting true when true has no effect). The two branches can be collapsed to a single block that is simpler and has the same behavior:

```rust
if il_pct > max_il {
    self.state.pause_flag = true;
    self.state.updated_at = Utc::now();
    return RiskAction::PauseRebalancing { il_pct };
}
```

This simplification is safe because the plan explicitly states "If `il_pct > max_il` and IS already paused: return `PauseRebalancing { il_pct }` (propagate)" — which is what the collapsed form does.

**S-02: `updated_at` is updated even when `halt_flag` was already set (entry-guard path)**

File: `src/strategy/risk_monitor.rs`, lines 352–357

When `halt_flag` is true and the halt gate fires at the top of `evaluate()`, the code still mutates `self.state.updated_at = Utc::now()` before returning. This causes a DB persist on every tick while halted, which is by design (fire-and-forget is cheap). However, the timestamp is being updated with no meaningful state change: `peak_pnl`, `current_drawdown_pct`, `pause_flag`, and `halt_flag` are not touched. The `persist_state` call in the caller will then write a row that is identical to the previous one except for `updated_at`. This is not incorrect, but it generates unnecessary DB writes in the permanently-halted state. A guard such as `if self.state.halt_flag { return RiskAction::HaltAll { ... }; }` (without updating `updated_at`) would allow the caller to skip the persist if the state hasn't changed, though this would require the caller to track that. The current behavior is acceptable; this is a low-priority observation.

**S-03: The `[ASSUMED]` annotation on the Drift program ID warrants a stronger action item**

File: `src/strategy/risk_monitor.rs`, line 205

The comment `[ASSUMED] Program ID correct as of training data; verify against official Drift docs before deployment` is correct to flag. However, the Drift v2 mainnet program ID `dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH` should be verified against the current Drift docs/on-chain data before LIVE-02 activates keypair-based Drift monitoring. Consider adding this as a concrete item to the LIVE-02 plan rather than leaving it only as an inline comment, since the comment will be invisible in the LIVE-02 planning context.

**S-04: The `drift_min_margin_ratio` CLI flag doc comment says "as a percentage" but the code uses it as a ratio**

File: `src/main.rs`, line 80

The arg comment says `e.g. 20.0 = 20%` but the `evaluate()` method compares `ratio < min_ratio` where `fetch_drift_margin_ratio()` returns a proxy ratio computed as `|quote| / (|base| + 1)`, which is not a percentage. The validation in the Watch arm only checks `> 0.0` with no upper bound, so a user passing `--drift-min-margin-ratio 20.0` thinking they mean 20% would actually set a ratio threshold of 20.0 (which would almost certainly never be met by the proxy calculation). When LIVE-02 revisits this, the semantics of the threshold vs. the computed value need to be made consistent. This is a documentation-versus-semantics mismatch that should be resolved before the flag has any real effect.

---

## Plan Deviation Analysis

**Deviation D-A: OrcaExecutor::execute_close_position does not exist — LP close deferred**

Recorded in 06-03-SUMMARY.md as an auto-fixed issue. The plan spec referenced `OrcaExecutor::execute_close_position` and `execute_collect_fees` for the drawdown breach path, but `src/execution/orca_executor.rs` does not exist in the Phase 6 codebase. The implementation fell back to logging both LP close and Drift hedge close as deferred to LIVE-02.

Assessment: This deviation is correctly handled. The plan's own fallback guidance explicitly allowed this: "If the executor ref is not available at that scope... log that LP close will be attempted on the next tick when the halt_flag check fires again." The halt_flag is persisted to DB, so rebalancing is correctly and permanently suppressed. The missing CPI is a gap in the execution layer, not in the risk monitoring layer. LIVE-02 should explicitly track this as a prerequisite.

**Deviation D-B: drift_user_pubkey set to None in Phase 6**

Recorded in 06-03-SUMMARY.md. The plan assumed keypair wiring would be available for `derive_drift_user_pda`. It is not present in Phase 6. `drift_user_pubkey = None` causes `fetch_drift_margin_ratio()` to short-circuit correctly, effectively disabling Drift margin monitoring in Phase 6.

Assessment: Correctly handled. RISK-03 in Phase 6 scope is "logs CRITICAL" only, with CPI deferred to LIVE-02. The short-circuit behavior is the documented Pitfall 5 mitigation (shadow mode / no keypair). No correctness impact.

---

## Must-Have Verification

| Must-have | Status | Notes |
|-----------|--------|-------|
| evaluate() returns correct RiskAction for each breach | PASS | All branches covered by unit tests |
| Drawdown check skipped when peak_pnl <= 0 | PASS | Guard present at line 366; test `drawdown_skipped_when_peak_not_positive` |
| IL percentage uses abs(il_usd) / position_value | PASS | Line 381; test `il_breach_returns_pause_rebalancing` |
| Evaluation order: halt -> drawdown -> IL -> Drift -> Continue | PASS | Lines 352-419; test `drawdown_fires_before_il_check` |
| Drift margin check returns Continue when drift_min_margin_ratio is None | PASS | Lines 406-415; test `drift_min_margin_none_returns_continue` |
| risk_state table with pool_address PRIMARY KEY | PASS | schema.sql line 68 |
| RiskState loaded from DB; missing row creates fresh state | PASS | load_or_init() lines 103-156 |
| RiskState persisted via fire-and-forget spawn | PASS | persist_state() lines 167-198 |
| halt_flag=true loaded from DB is preserved | PASS | SELECT-then-INSERT pattern; tracing::error! on halt detection |
| Drift RPC failure treated as margin OK with warning | PASS | Lines 261-269; test `fetch_drift_margin_ratio_returns_none_when_no_pubkey` |
| Risk monitor loads state from DB at watch startup | PASS | main.rs line 556 |
| Risk monitor evaluates on every tick after pnl_write | PASS | main.rs lines 761-853; D-05 order confirmed |
| CLI accepts --max-drawdown, --max-il, --drift-min-margin-ratio | PASS | main.rs lines 75-83 with #[arg(long)] |
| Risk state persisted after each evaluate() | PASS | All five RiskAction arms call persist_state |
| HaltAll: halt_flag persisted, tick cycle skipped | PASS | main.rs lines 782-800 |
| IL breach skips should_rebalance | PASS | main.rs line 814: return |
| Drift margin breach logs CRITICAL, LP rebalance continues | PASS | main.rs lines 829-842: fall through |

---

## Summary

Phase 06 is well-implemented. The core state machine logic (`evaluate()`) is correct, the DB persistence pattern is safe (particularly the halt_flag preservation on restart), and the watch-loop wiring respects the D-05 ordering constraint. The test suite is comprehensive.

**Two items warrant follow-up before LIVE-02 activates Drift monitoring:**

1. I-01: Replace `lock().unwrap()` with a panic-safe lock acquisition pattern throughout `src/main.rs` in the risk gate section.
2. I-02: The Drift account byte-offset fallback currently returns `Some(0.0)` instead of `None` when the account is shorter than `PERP_ARRAY_OFFSET + 8*136` bytes, which would incorrectly trigger `CloseDriftHedge` on every tick with a real Drift account. Add an explicit `return None` when the length guard fails.

Neither I-01 nor I-02 affects Phase 6 functionality today (the Drift path is effectively disabled by `drift_user_pubkey = None`), but both must be resolved before LIVE-02 enables keypair-based Drift monitoring.
