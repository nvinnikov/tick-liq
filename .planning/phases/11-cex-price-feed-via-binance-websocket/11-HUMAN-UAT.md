---
status: resolved
phase: 11-cex-price-feed-via-binance-websocket
source: [11-VERIFICATION.md]
started: 2026-04-17T00:00:00Z
updated: 2026-04-17T00:00:00Z
---

## Current Test

Resolved via Plan 02 checkpoint sign-off (user replied "approved").

## Tests

### 1. Live Binance feed — price matches within ~5s
expected: on-screen Price matches Binance SOL/USDT mid within a few cents; logs show connect sequence
result: passed (checkpoint approved)

### 2. No-flag behavior — on-chain fallback
expected: logs contain "--cex-symbol not set, using on-chain price"; behavior identical to pre-Phase-11
result: passed (checkpoint approved)

### 3. Stale transition logging — exactly one warn/info per transition
expected: exactly ONE warn on disconnect >30s; exactly ONE info on reconnect recovery
result: passed (checkpoint approved)

### 4. Ctrl+C graceful shutdown — both WS tasks exit cleanly
expected: both "cex_ws: clean shutdown" and "WS watch: clean shutdown" within ~1s
result: passed (checkpoint approved)

### 5. DB price column — pnl_history.price reflects Binance mid
expected: SELECT price FROM pnl_history shows Binance mid, not on-chain derivation
result: passed (checkpoint approved)

## Summary

total: 5
passed: 5
issues: 0
pending: 0
skipped: 0
blocked: 0

## Gaps
