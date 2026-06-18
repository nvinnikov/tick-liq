# Prometheus / VictoriaMetrics observability — design

**Date:** 2026-06-17
**Branch:** prom-metrics
**Status:** approved (proceeding to implementation)

## Goal

Real-time and post-hoc observability of the LP manager. Two things matter:

1. **Price reconciliation** — compare mid-prices across sources (Binance, Orca,
   and a newly added Coinbase feed) and surface the deviation, so we can see when
   the on-chain Orca price drifts from CEX reference prices.
2. **Position state** — expose live PnL / IL / fees / greeks / in-range / rebalance
   signal so a dashboard reflects the running `watch` session.

The mechanism must work in **both** runtime modes:

- a **long-running** `watch` on a server (24/7), and
- **short, manual CLI runs** (minutes/hours) where a pull-scrape never lands.

## Non-goals (v1 / YAGNI)

- No histograms — every metric is a point-in-time gauge.
- No committed Grafana dashboards — we publish metric *names* only.
- No quantiles/summaries.
- No change to the existing Postgres/TimescaleDB write path. Metrics are a
  *separate, additive* observability channel, not a replacement for persistence.

## Transport (decision: `metrics` + `metrics-exporter-prometheus`)

One global in-process recorder. Metric emission on the hot path is an atomic
`gauge!(...).set(x)` — near-zero cost; exposition happens only at scrape/push
time. Mode is chosen from env at `watch` start:

| Env | Mode | Behaviour |
|-----|------|-----------|
| `METRICS_LISTEN=0.0.0.0:9100` | pull | `.with_http_listener(addr).install()` — Prometheus/VictoriaMetrics scrape `/metrics`. For the long-running server. |
| `METRICS_PUSH_URL=<url>` + `METRICS_PUSH_INTERVAL_SECS=<n>` | push | `.with_push_gateway(url, interval, …).install()` — the exporter POSTs rendered metrics to VictoriaMetrics `/api/v1/import/prometheus`. For short CLI runs. |
| neither set | off | recorder not installed → `metrics` macros are no-ops. Zero overhead, `watch` behaviour unchanged (important for tests/backtests). |

Both modes emit identical metric code in the watch loop; only init differs. No
extra HTTP-client dependency — the exporter crate performs the push itself.

If both env vars are set, `METRICS_LISTEN` wins (log a warn that push is ignored).

## Components (isolated units)

### 1. `src/metrics/mod.rs` — observability transport
- `init_from_env() -> anyhow::Result<Option<Handle>>` — inspects env, picks
  listener / push / off, installs the recorder, calls all `describe_*`. Returns
  `Ok(None)` when off.
- Thin emit helpers, source-agnostic:
  - `record_price(source: Source, mid: f64)`
  - `record_deviation(source: Source, bps: f64)`
  - `record_feed(source: Source, up: bool, staleness_secs: f64)`
  - `record_position(snap: &PositionMetrics)` where `PositionMetrics` is a small
    plain struct holding the values the watch loop already computes.
- `deviation_bps(price: f64, reference: f64) -> Option<f64>` — pure; returns
  `None` when `reference <= 0` or inputs non-finite.
- Depends only on `metrics`, `metrics-exporter-prometheus`. No knowledge of RPC,
  pools, or the DB.

### 2. Price-feed generalization
- `enum Source { Binance, Coinbase, Orca }` with a `&'static str` label (used as
  the `source=` metric label). Lives where the feeds can share it (e.g.
  `src/data/mod.rs`).
- Extract the quote-sanity check out of `cex_ws.rs::apply_book_ticker` into a
  shared `validate_quote(bid: f64, ask: f64) -> bool` (finite, positive,
  `bid <= ask`). Binance keeps using it; Coinbase reuses it. Existing
  `apply_book_ticker` tests stay green.
- `src/data/coinbase_ws.rs` — new feed on the existing `tokio-tungstenite`
  dependency (no new SDK). Connects to `wss://ws-feed.exchange.coinbase.com`,
  subscribes to the `ticker` channel for the configured product (e.g.
  `SOL-USD`), parses `best_bid`/`best_ask`, validates via `validate_quote`,
  writes mid into its shared state slot. Mirrors `cex_ws.rs`: connect-retry
  backoff (`CONNECT_RETRY_BASE`..`MAX`), in-session liveness watchdog
  (`FEED_STALE_TIMEOUT`), shutdown via broadcast. Each accepted quote also calls
  `metrics::record_price(Source::Coinbase, mid)`.

### 3. Deviation
- Computed in the watch loop where Binance, Orca, and (optionally) Coinbase mids
  are all in hand. `record_deviation(Source::Orca, bps)` and
  `record_deviation(Source::Coinbase, bps)`, reference = Binance mid (the
  `ref="binance"` label). Skipped when Binance mid is unavailable.

### 4. Watch-loop wiring (`src/main.rs`)
- Call `metrics::init_from_env()` at `watch` start; log chosen mode.
- Spawn the Coinbase feed alongside the Binance feed, gated by `COINBASE_SYMBOL`
  (or a `--coinbase-symbol` flag); off when unset.
- After the per-tick PnL/IL/greeks computation that already happens, build a
  `PositionMetrics` and call `metrics::record_position(...)`, plus deviation +
  feed-health emits.

## Metric set (prefix `tickliq_`)

Prices / feeds:
- `tickliq_price_mid{source}` — gauge, mid-price per source (`binance|orca|coinbase`).
- `tickliq_price_deviation_bps{source,ref="binance"}` — gauge, signed bps of `source` vs reference.
- `tickliq_feed_up{source}` — gauge 0/1.
- `tickliq_feed_staleness_seconds{source}` — gauge, seconds since last accepted quote.

Position (label `mint`, bounded by number of watched positions):
- `tickliq_position_value_usd`
- `tickliq_pnl_usd` (real PnL = fees − IL)
- `tickliq_fees_earned_usd`
- `tickliq_il_usd`
- `tickliq_delta`, `tickliq_gamma`
- `tickliq_in_range` (0/1)
- `tickliq_rebalance_signal` (0/1)

Label cardinality is bounded (3 sources, few mints) — safe for Prometheus.

## Error handling

Metrics must never affect `watch` correctness:
- `init_from_env` failure → `warn!` and continue with metrics off.
- Push failures → exporter logs/retries on the next interval; never propagated.
- All emit helpers are infallible (gauge sets).
- The Coinbase feed fails and reconnects in isolation, exactly like Binance; a
  dead Coinbase feed must not stall Binance or the watch loop.

## Testing

- `deviation_bps`: sign, magnitude, and guards (`reference <= 0`, non-finite → `None`).
- Coinbase ticker parse + `validate_quote`: reuse the `apply_book_ticker` test
  matrix (missing/non-numeric/NaN/inf/negative/zero/crossed book → rejected,
  state unchanged).
- Render smoke test: after `record_price` / `record_position` against an
  installed recorder, `handle.render()` contains the expected metric names/labels.
- Metrics-off path: emit helpers are safe no-ops; no panic, no behaviour change.

## Config / docs

Env-driven (consistent with the project's env-only secret/config policy):
`METRICS_LISTEN`, `METRICS_PUSH_URL`, `METRICS_PUSH_INTERVAL_SECS`,
`COINBASE_SYMBOL`. Update the **Environment** section of `README.md`.

## Dependencies added

- `metrics` (facade)
- `metrics-exporter-prometheus` (HTTP listener + push-gateway features)

No new transport/TLS deps; Coinbase reuses `tokio-tungstenite`.
