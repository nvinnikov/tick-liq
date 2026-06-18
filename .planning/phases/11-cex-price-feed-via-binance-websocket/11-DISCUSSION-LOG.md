# Phase 11: CEX price feed via Binance WebSocket - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-17
**Phase:** 11-cex-price-feed-via-binance-websocket
**Areas discussed:** Connection & Library, Shared State, Fallback on Disconnect, Symbol Configuration, Scope of Replacement

---

## Connection & Library

| Option | Description | Selected |
|--------|-------------|----------|
| raw tokio-tungstenite | Already a dependency, pattern exists in ws.rs — zero new crates | ✓ |
| binance crate (0.21.2) | Community crate with WS streams, adds dependency | |
| binance-sdk (official v45) | Official rewrite, Rust 2024 edition / resolver=3 — compatibility risk | |

**User's choice:** raw tokio-tungstenite  
**Notes:** User initially asked to check official Binance Rust libs. After reviewing both (`binance-sdk` v45 and `binance` 0.21.2), user confirmed raw tungstenite since the pattern already exists in the codebase.

---

## Shared State

| Option | Description | Selected |
|--------|-------------|----------|
| Arc<RwLock<Option<f64>>> | Option models "no price yet", readable, idiomatic Rust | ✓ |
| Arc<AtomicU64> (f64 bitcast) | Lock-free, slightly less readable | |
| tokio::watch::channel | Natural Tokio pattern, more boilerplate | |

**User's choice:** `Arc<RwLock<Option<f64>>>`

---

## Fallback on Disconnect

| Option | Description | Selected |
|--------|-------------|----------|
| Freeze rebalance decisions | No action if price stale | |
| Fallback to on-chain price | Use sqrt_price from pool if CEX unavailable | ✓ |
| Telegram alert + freeze | Notify operator via existing bot | |

**User's choice:** Fallback to on-chain price  
**Staleness threshold:** 30 seconds

**Notes:** User mentioned multi-exchange fallback (Kraken/Bybit) as a future idea — noted as deferred.

---

## Symbol Configuration

| Option | Description | Selected |
|--------|-------------|----------|
| CLI flag --cex-symbol | watch --cex-symbol SOLUSDT, flexible | ✓ |
| Hardcode SOLUSDT | Simpler, single-pool lock-in | |
| env var CEX_SYMBOL | Consistent with WALLET_KEYPAIR pattern | |

**User's choice:** CLI flag `--cex-symbol`

---

## Scope of Replacement

| Option | Description | Selected |
|--------|-------------|----------|
| Rebalance signal only | Only tick_current comparison | |
| Rebalance signal + IL | Signal and IL calculation | |
| Rebalance signal + IL + P&L DB | All three — full replacement | ✓ |

**User's choice:** All three (rebalance signal, IL calculation, P&L DB price column)

---

## Deferred Ideas

- Multi-exchange price fallback (Binance → Kraken/Bybit) — future phase
- Telegram alert on CEX disconnect — future phase
