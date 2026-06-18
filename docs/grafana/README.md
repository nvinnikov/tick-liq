# Grafana dashboard

`tick-liq-overview.json` — обзорный дашборд для метрик, которые `watch` отдаёт через
Prometheus/VictoriaMetrics (см. `docs/superpowers/specs/2026-06-17-prom-metrics-design.md`).

## Импорт

1. Grafana → **Dashboards → New → Import → Upload JSON file** → выбрать `tick-liq-overview.json`.
2. В диалоге выбрать datasource (переменная `Datasource`, тип `prometheus`).
   VictoriaMetrics Prometheus-совместима — подходит тот же тип datasource.
3. Переменная `Position mint` подтягивается через `label_values(tickliq_position_value_usd, mint)`
   и поддерживает мультивыбор.

## Где брать метрики

- **Pull (долгий `watch`):** `METRICS_LISTEN=0.0.0.0:9100` → Prometheus/VictoriaMetrics
  скрейпят `http://<host>:9100/metrics`.
- **Push (короткие прогоны):** `METRICS_PUSH_URL=http://<vm>:8428/api/v1/import/prometheus`
  (+ опц. `METRICS_PUSH_INTERVAL_SECS`).

## Панели

- **Prices & divergence** — `tickliq_price_mid{source}` по трём источникам;
  `tickliq_price_deviation_bps{source,ref="binance"}` (Orca-vs-Binance = расхождение on-chain↔CEX,
  пороги 50/100 bps).
- **Feed health** — `tickliq_feed_up{source}` (state-timeline) и
  `tickliq_feed_staleness_seconds{source}` (сентинел «never populated» = `f64::MAX` отфильтрован
  условием `< 1e18`).
- **Position P&L** — стат-панели value/pnl/fees/IL, разложение P&L во времени,
  greeks (delta/gamma), и state-timeline in-range/rebalance-signal. Все фильтруются по `$mint`.
