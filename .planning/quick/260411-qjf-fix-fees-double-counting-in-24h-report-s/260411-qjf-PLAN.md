---
phase: quick-260411-qjf
plan: 01
type: execute
wave: 1
depends_on: []
files_modified:
  - src/bot/queries.rs
autonomous: true
must_haves:
  truths:
    - "24H report shows actual fee delta over the period, not cumulative sum"
    - "24H report shows actual IL delta over the period, not cumulative sum"
    - "24H report shows actual net P&L delta over the period, not cumulative sum"
    - "Price range (earliest/latest) still works correctly"
  artifacts:
    - path: "src/bot/queries.rs"
      provides: "Corrected 24H report SQL using MAX-MIN delta instead of SUM"
      contains: "MAX(fees_earned)"
  key_links:
    - from: "src/bot/commands.rs"
      to: "src/bot/queries.rs"
      via: "query_24h_report()"
      pattern: "query_24h_report"
---

<objective>
Fix double-counting bug in the 24H report SQL query.

Purpose: `pnl_history` stores cumulative snapshots (fees_earned, il_usd, net_pnl) on every tick. The current query uses SUM() across all rows in the 24h window, which sums the same accumulated value thousands of times. For example, $0.0082 in actual fees becomes ~$8.9 when summed across 2705 rows.

Output: Corrected SQL that computes the actual delta over the reporting period using MAX() - MIN() for all three cumulative fields.
</objective>

<execution_context>
@$HOME/.claude/get-shit-done/workflows/execute-plan.md
@$HOME/.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@src/bot/queries.rs
@src/bot/commands.rs
</context>

<interfaces>
<!-- From src/bot/queries.rs — the struct consumed by commands.rs -->
```rust
pub struct ReportData {
    pub total_fees: f64,
    pub total_il: f64,
    pub total_net_pnl: f64,
    pub row_count: i64,
    pub earliest_price: f64,
    pub latest_price: f64,
}
```

<!-- From src/bot/commands.rs:171 — the caller -->
```rust
match super::queries::query_24h_report(&state.db_pool, &state.pool_address).await {
```
</interfaces>

<tasks>

<task type="auto">
  <name>Task 1: Fix 24H report SQL — replace SUM with MAX-MIN delta</name>
  <files>src/bot/queries.rs</files>
  <action>
In `query_24h_report` (line 75-101), replace the SQL query. All three cumulative fields must use MAX() - MIN() to compute the actual delta over the 24h window:

Change the SELECT from:
```sql
COALESCE(SUM(fees_earned), 0.0) AS total_fees,
COALESCE(SUM(il_usd), 0.0) AS total_il,
COALESCE(SUM(net_pnl), 0.0) AS total_net_pnl,
```

To:
```sql
COALESCE(MAX(fees_earned) - MIN(fees_earned), 0.0) AS total_fees,
COALESCE(MAX(il_usd) - MIN(il_usd), 0.0) AS total_il,
COALESCE(MAX(net_pnl) - MIN(net_pnl), 0.0) AS total_net_pnl,
```

The price fields (MIN/MAX with FILTER) and COUNT(*) remain unchanged — they are already correct.

The `ReportData` struct and `commands.rs` consumer require NO changes — the field names and types are identical.

Note on IL: il_usd is also cumulative (written from the same snapshot in writer.rs:96), so it has the same double-counting bug and needs the same fix. net_pnl = fees_earned - il_usd, also cumulative, same fix.
  </action>
  <verify>
    <automated>cd /Users/n.vinnikov/PycharmProjects/tick-liq && cargo build 2>&1 | tail -5</automated>
  </verify>
  <done>
    - SQL uses MAX()-MIN() for fees_earned, il_usd, and net_pnl
    - No SUM() on any cumulative field
    - Project compiles without errors
    - ReportData struct unchanged, commands.rs untouched
  </done>
</task>

</tasks>

<threat_model>
## Trust Boundaries

No new trust boundaries introduced — this is a SQL logic fix within an existing query. All inputs already parameterized ($1 bind).

## STRIDE Threat Register

| Threat ID | Category | Component | Disposition | Mitigation Plan |
|-----------|----------|-----------|-------------|-----------------|
| T-qjf-01 | T (Tampering) | queries.rs SQL | accept | Query already uses bind parameters (no injection risk). Change is aggregation logic only. |
</threat_model>

<verification>
1. `cargo build` succeeds
2. `cargo clippy -- -D warnings` passes
3. Grep confirms no remaining `SUM(fees_earned)`, `SUM(il_usd)`, or `SUM(net_pnl)` in queries.rs
</verification>

<success_criteria>
- The 24H report query computes deltas (MAX-MIN) instead of sums for all cumulative fields
- Build and clippy pass
- When deployed, /report will show actual fee/IL/PnL deltas matching the on-chain values visible in /status
</success_criteria>

<output>
After completion, create `.planning/quick/260411-qjf-fix-fees-double-counting-in-24h-report-s/260411-qjf-SUMMARY.md`
</output>
