# CONCERNS.md — Technical Debt & Concerns

## Current State

The codebase is in pre-implementation phase. All concerns listed below are **pre-emptive** based on the planned architecture, not observed bugs.

---

## High-Priority Concerns

### 1. No Source Code Yet
**Risk:** All structure is planned but unimplemented. The project spec in `CLAUDE.md` is the only artifact.
**Impact:** Nothing to test, nothing to validate math against real on-chain data.

### 2. Key Management
**Risk:** Any mistake storing or logging keypairs means funds loss.
**Mitigation:** Env vars only; add a startup check that refuses to run if keypair appears in config file; never log pubkeys in combination with amounts in a way that reveals wallet identity.

### 3. Solana Account Deserialization
**Risk:** Orca Whirlpool and Raydium CLMM account layouts can change across program upgrades.
**Mitigation:** Always pin the protocol crate version and test deserialization against known account snapshots. Verify `account.owner` before any deserialization attempt.

---

## Medium-Priority Concerns

### 4. RPC Rate Limits & Reliability
**Risk:** Free-tier RPC nodes will throttle under WebSocket subscriptions + polling.
**Mitigation:** Connection pool; exponential backoff on errors; Helius/QuickNode paid plans for production.

### 5. WebSocket Reconnection
**Risk:** Dropped WebSocket means missed pool updates — position state goes stale.
**Mitigation:** Reconnect with exponential backoff; periodic full state refresh via HTTP RPC as fallback.

### 6. Rebalance Transaction Failure
**Risk:** Close position succeeds but open new position fails — funds left undeployed or in wrong state.
**Mitigation:** Check transaction simulation before submission; atomic where possible; monitoring alert if position is closed but not reopened within N seconds.

### 7. Math Precision (f64 vs integer arithmetic)
**Risk:** CLMM math on real Solana uses Q64.64 fixed-point. Using `f64` may diverge from on-chain results at extremes.
**Mitigation:** Use `proptest` to compare against reference JS SDK; consider wrapping in a `Price` newtype to make unit explicit; evaluate `rust_decimal` or Q64.64 if precision issues emerge.

---

## Low-Priority / Future Concerns

### 8. Backtester Data Quality
**Risk:** Orca historical swap API may have gaps; parsing raw transactions is fragile.
**Action needed:** Identify reliable historical data source before starting Phase 4.

### 9. Drift Protocol Integration Complexity
**Risk:** Drift v2 has complex margin/liquidation mechanics that interact poorly with automated hedging.
**Action needed:** Scope Phase 6 carefully; paper-trade the hedge before live deployment.

### 10. No Observability Infrastructure
**Risk:** In production, hard to debug why a rebalance didn't trigger or why P&L diverges.
**Mitigation:** `tracing` is in the stack — ensure spans are used at decision boundaries (rebalance signal evaluation, transaction submission). Plan Grafana/metrics endpoint before mainnet.

### 11. Single-Keypair Risk
**Risk:** Bot uses one keypair for all transactions — compromise = full loss.
**Future:** Consider multi-sig or hardware wallet for large positions.

---

## Dependency Risks

| Crate | Risk |
|-------|------|
| `solana-client 1.18` | Solana major version upgrades are breaking |
| `anchor-client 0.29` | Anchor upgrade requires recompiling against new IDL |
| `sqlx 0.7` | TimescaleDB extensions may lag behind PostgreSQL major versions |
| `tokio-tungstenite 0.21` | Solana WebSocket quirks may require workarounds |
