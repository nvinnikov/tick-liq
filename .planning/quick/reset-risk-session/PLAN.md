---
type: execute
autonomous: true
files_modified:
  - src/strategy/risk_monitor.rs
  - src/main.rs
---

<objective>
Reset `peak_pnl` and `halt_flag` to `0.0`/`false` at the start of every new watch session so stale DB values from a previous session do not produce an immediate 100% drawdown halt on restart.

Purpose: On restart, `net_pnl` starts at 0 but `peak_pnl` loaded from DB may be positive, making `drawdown = (peak - 0) / peak * 100 = 100%` and immediately halting the bot.
Output: `RiskMonitor::reset_session()` method + call in `main.rs` after `load_or_init`.
</objective>

<context>
@src/strategy/risk_monitor.rs
@src/main.rs
</context>

<tasks>

<task type="auto" status="completed">
  <name>Task 1: Add reset_session() to RiskMonitor and call it from main.rs</name>
  <files>src/strategy/risk_monitor.rs, src/main.rs</files>
  <action>
In `src/strategy/risk_monitor.rs`, add a new `async fn reset_session` to `RiskMonitor` (or as an associated function taking `&PgPool` and `pool_address`):

```rust
/// Reset volatile session state so a fresh watch session starts with a clean slate.
/// Zeroes `peak_pnl` and `halt_flag`; preserves `operator_pause` (it is intentional).
/// Persists immediately via an UPDATE so the next restart also starts clean.
pub async fn reset_session(pool: &PgPool, pool_address: &str) -> anyhow::Result<()> {
    pool.execute(
        query(
            "UPDATE risk_state \
             SET peak_pnl = 0.0, halt_flag = FALSE, \
                 current_drawdown_pct = 0.0, updated_at = NOW() \
             WHERE pool_address = $1",
        )
        .bind(pool_address),
    )
    .await
    .map_err(|e| anyhow::anyhow!("reset_session UPDATE failed: {}", e))?;
    Ok(())
}
```

Place it just after `load_or_init` (around line 160). It does NOT touch `operator_pause` or `pause_flag`; those can remain for operator control.

In `src/main.rs`, call `reset_session` immediately after `load_or_init` succeeds (around line 594), before the `halt_flag` log check:

```rust
let risk_state =
    strategy::risk_monitor::RiskMonitor::load_or_init(pg, &pool_addr).await?;

// Reset session-volatile fields so stale peak_pnl from prior session
// does not produce instant 100% drawdown on restart (bug fix).
strategy::risk_monitor::RiskMonitor::reset_session(pg, &pool_addr).await?;

// Re-load to pick up the zeroed values.
let risk_state =
    strategy::risk_monitor::RiskMonitor::load_or_init(pg, &pool_addr).await?;
```

Alternatively (simpler): after calling `reset_session`, manually zero the already-loaded `risk_state`:

```rust
let mut risk_state =
    strategy::risk_monitor::RiskMonitor::load_or_init(pg, &pool_addr).await?;

strategy::risk_monitor::RiskMonitor::reset_session(pg, &pool_addr).await?;

// Apply zeroed values to the in-memory state without a second DB round-trip.
risk_state.peak_pnl = 0.0;
risk_state.halt_flag = false;
risk_state.current_drawdown_pct = 0.0;
```

Use this second (simpler) approach to avoid an extra DB SELECT.

The existing `halt_flag` warning log at line 597 of `main.rs` should be removed or moved above the reset call if it is meant to inform the user that a halt was detected and is now being cleared. Log at INFO level: `"risk: session reset — peak_pnl and halt_flag cleared for new session"`.
  </action>
  <verify>cargo build 2>&1 | grep -E "^error" | head -20 && cargo test -p tick-liq risk_monitor 2>&1 | tail -20</verify>
  <done>
    - `cargo build` succeeds with no errors.
    - `cargo test risk_monitor` passes all existing tests.
    - On a watch restart with a non-zero `peak_pnl` in DB, the first `evaluate()` call sees `peak_pnl = 0` and does not emit `HaltTrading`.
  </done>
</task>

<task type="auto" tdd="true" status="completed">
  <name>Task 2: Unit test — reset_session zeroes session fields</name>
  <files>src/strategy/risk_monitor.rs</files>
  <behavior>
    - Test: after calling `reset_session` (mocked or via sqlx::test), `peak_pnl == 0.0`, `halt_flag == false`, `current_drawdown_pct == 0.0`.
    - Test: `operator_pause` is unchanged after `reset_session`.
    - Existing drawdown test with `peak_pnl > 0` should still work (evaluating after a reset would start from 0, not trigger halt).
  </behavior>
  <action>
Add a unit test in the existing `#[cfg(test)]` block of `risk_monitor.rs`. Since `reset_session` requires a real `PgPool`, test the in-memory state path:

```rust
#[test]
fn new_session_start_does_not_halt_when_pnl_zero() {
    // Simulate post-reset state: peak_pnl=0, halt_flag=false
    let state = make_state("POOL", 0.0, false, false);
    let mut rm = monitor_all(state, Some(50.0), None, None);
    let snap = make_snap(0.0, 0.0, 1000.0); // net_pnl=0 at session start
    let action = rm.evaluate(&snap, None);
    assert_eq!(action, RiskAction::Continue, "zero peak_pnl must never trigger halt");
}
```

This verifies the fix end-to-end at the logic level without needing a DB.
  </action>
  <verify>cargo test -p tick-liq new_session_start_does_not_halt 2>&1 | tail -10</verify>
  <done>New test passes; `cargo test risk_monitor` green.</done>
</task>

</tasks>

<success_criteria>
- `cargo build` clean.
- `cargo test risk_monitor` all green including new test.
- Restarting watch with a DB row where `peak_pnl = 500.0, halt_flag = true` results in the session starting with `peak_pnl = 0.0, halt_flag = false` and no immediate halt action.
</success_criteria>
