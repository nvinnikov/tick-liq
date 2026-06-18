---
status: testing
phase: 11-cex-price-feed-via-binance-websocket
source: [11-01-SUMMARY.md, 11-02-SUMMARY.md]
started: 2026-04-17T00:00:00Z
updated: 2026-04-17T00:00:00Z
---

## Current Test

number: 1
name: Cold Start Smoke Test
expected: |
  Kill any running watch process. Run `cargo build` from scratch.
  Then: `cargo run -- watch --help`
  Expected: builds without errors, help output includes `--cex-symbol <SYMBOL>` flag.
awaiting: user response

## Tests

### 1. Cold Start Smoke Test
expected: Kill any running watch process. Run `cargo build` from scratch. Then `cargo run -- watch --help`. Builds without errors, help output includes `--cex-symbol <SYMBOL>` flag.
result: [pending]

### 2. CEX feed active — price tracks Binance
expected: Run `cargo run -- watch --cex-symbol SOLUSDT ...`. Within ~5s logs show Binance connect sequence and price values. On-screen price matches Binance SOL/USDT mid within a few cents.
result: [pending]

### 3. No --cex-symbol — on-chain fallback
expected: Run without `--cex-symbol` flag. Logs contain `--cex-symbol not set, using on-chain price`. Behavior identical to pre-Phase-11 (no crash, normal watch loop).
result: [pending]

### 4. Empty --cex-symbol — immediate error
expected: Run `cargo run -- watch --cex-symbol "" ...`. Process exits immediately with a clear error message about empty symbol (not a crash/panic, and no reconnect loop).
result: [pending]

### 5. Stale feed logging — one warn per transition
expected: Disconnect network or kill Binance feed for >30s. Exactly ONE warn log line about stale feed (not repeated every tick). On reconnect, exactly ONE info line confirming fresh feed restored.
result: [pending]

### 6. Ctrl+C graceful shutdown
expected: While watch is running with `--cex-symbol`, press Ctrl+C. Both "cex_ws: clean shutdown" and watch shutdown messages appear within ~1s. No hanging process.
result: [pending]

### 7. DB price column — reflects Binance mid
expected: After a few P&L ticks with `--cex-symbol SOLUSDT`, query `SELECT price FROM pnl_history ORDER BY recorded_at DESC LIMIT 5`. Prices shown match the Binance SOL/USDT mid, not the on-chain sqrt_price derivation.
result: [pending]

## Summary

total: 7
passed: 0
issues: 0
pending: 7
skipped: 0
blocked: 0

## Gaps

