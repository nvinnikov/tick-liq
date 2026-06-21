# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Automated LP (Liquidity Provider) Manager for concentrated liquidity pools on Solana. Reads on-chain CLMM positions (Orca Whirlpools / Raydium CLMM), calculates real-time P&L (fees earned minus impermanent loss), option-style Greeks (delta/gamma), price impact and tick-level liquidity depth, generates range-rebalance and delta-hedge signals, and ships an offline CLMM backtester built on the same math as the live inspector.

The binary is **`lp-inspect`** (the package/lib is `tick-liq`/`tick_liq`). Rust edition **2024**, MSRV **1.86** (required by `binance-sdk` 45).

> **Status: research / educational, dry-run only.** Execution paths (`rebalance`, `hedge`) build and print plans but **send no transactions and no CPI** — submission is gated behind `ShadowGuard` and a phased rollout that has not been enabled. Treat this as a portfolio/research project, not a turnkey trading system.

## Commands

```bash
# Build / test / lint / format
cargo build
cargo test                      # unit + property + golden (DB integration tests are #[ignore]'d)
cargo test <test_name>          # run a single test
cargo test --test math_props    # proptest property-based invariants
cargo test --test math_golden   # golden vectors vs Orca SDK reference
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt

# Makefile shortcuts (load .env, manage the Postgres docker-compose stack)
make up / make down             # start / stop local Postgres+TimescaleDB
make run ARGS="position <MINT>"
make migrate                    # cargo run -- db migrate, with .env loaded
```

The CLI takes the position **MINT** (or pool address) as a *positional* argument, not a `--mint`/`--pool` flag:

```bash
# binary is `lp-inspect`; --rpc-url / --db-url / --rpc-timeout are global flags (or env)
cargo run -- position <POSITION_MINT> [--protocol orca|raydium] [--entry-price <P>]
cargo run -- watch <POSITION_MINT> [--live] [--telegram] [--cex-symbol SOLUSDT] \
    [--max-drawdown 10] [--max-il 5] [--drift-min-margin-ratio 20 --drift-authority <PUBKEY>] \
    [--coinbase-symbol SOL-USD] [--entry-price <P>]
cargo run -- depth <POOL_ADDRESS>
cargo run -- impact <POOL_ADDRESS> --size <USD>
cargo run -- strategy check <POSITION_MINT> [--near-edge-ticks 10] [--min-pnl 0] [--entry-price <P>]
cargo run -- rebalance <POSITION_MINT> --dry-run
cargo run -- hedge <POSITION_MINT> --dry-run
cargo run -- db migrate          # applies the embedded schema (idempotent)

# Backtesting (GBM synthetic path, or real-data replay)
cargo run -- backtest --entry-price 84 --price-lower 75 --price-upper 95 --days 30 --rebalance
cargo run -- backtest --pool <POOL_ADDRESS> --from 2026-01-01 --to 2026-01-15 \
    --entry-price 84 --price-lower 75 --price-upper 95 --decimals-a 9 --decimals-b 6  # DB replay
cargo run -- backfill --pool <POOL_ADDRESS> --from 2026-06-01 --to 2026-06-15 \
    --fee-bps 4 --pool-liquidity <L> [--dry-run]   # seed pool_ticks from GeckoTerminal OHLCV
cargo run -- research --config research/experiments.toml --out research/data/results.csv
```

Default `--rpc-url` is `https://api.devnet.solana.com`; `watch` derives its WebSocket URL by rewriting `https→wss`. Use a private RPC for `watch` (public endpoints rate-limit WS).

## Architecture

Layered around a pure math core. Library entry-point is `src/lib.rs`; the CLI lives in `src/main.rs` (a large dispatcher — shared helpers like `compute_orca_pnl`, `load_orca_position_and_pool`, `resolve_entry_price` live near the top).

```
src/
├── math/        Pure CLMM math — ZERO Solana/RPC/I/O deps (il, greeks, fees, impact, sqrt_price, metrics). Start here; fully testable.
├── protocols/   Borsh deserialization for Orca Whirlpool + Raydium CLMM (positions, pools, TickArrays). Owner-verified.
├── analytics/   Thin orchestration bridging protocols/ + math/ (amounts, greeks, pnl, depth); does the raw↔UI price conversions.
├── data/        WebSocket pool subscription (ws) with reconnect+ping/pong; CEX price feeds (cex_ws=Binance, coinbase_ws); geckoterminal OHLCV. `Source` enum labels metrics.
├── strategy/    Pure rebalance signal (signal::should_rebalance) + risk_monitor (drawdown halt / IL pause / Drift margin, persisted).
├── execution/   Dry-run rebalance planner + Drift hedge size estimator, behind shadow_guard::ShadowGuard (the submission chokepoint).
├── backtest/    GBM simulator + db_replay (real ticks) + backfill (GeckoTerminal → pool_ticks). Runs the full data→math→signal pipeline offline.
├── storage/     Postgres + TimescaleDB (writer, tick_reader); schema embedded via include_str!(schema.sql), applied by `db migrate`.
├── bot/         Telegram operator bot (teloxide): rebalance approval flow, /pause /resume commands, allow-listed users.
├── metrics/     Prometheus/VictoriaMetrics exporter (pull via METRICS_LISTEN or push via METRICS_PUSH_URL). Self-contained, no-op when unconfigured.
├── display/     Formatted CLI tables + ASCII liquidity histogram.
├── cache.rs     Per-position metadata cache (entry price) under XDG data dir; mint validated as base58 before use as filename.
├── research.rs  In-memory pool × width × rebalance sweep → CSV (reuses backfill/db_replay/metrics; no DB).
└── rpc.rs       Blocking Solana RPC client: account fetch with owner verification, mint decimals, token symbol; retry w/ backoff.
```

`docs/` holds the research writeup, math-validation notes, and Grafana dashboard JSON. `.planning/` holds phase plans/summaries (historical context, not authoritative for current behavior). `research/` holds the experiment config and the Python analysis/charts.

## Key Technical Notes

- Use `anyhow` for error handling; **no `unwrap()`/`expect()`/`panic!` in production paths**.
- **Owner-verify before deserializing.** Accounts are parsed directly with `borsh` (no `anchor-client`): skip the 8-byte discriminator, verify the program owner via `rpc::SolanaRpc::fetch_account_checked`. Math is validated against the [Orca Whirlpool JS SDK](https://github.com/orca-so/whirlpools) / `orca_whirlpools_core`.
- **Math purity.** `src/math/` must stay free of Solana/protocol/RPC/I/O deps — tick↔sqrt-price and on-chain conversions belong in `analytics`. See `src/math/CLAUDE.md`.
- **Unit-space discipline (BUG-qr9 class).** Two price spaces exist: *raw* (token-B base units per token-A base unit) and *UI* (decimal-adjusted). Every IL/P&L/entry-price comparison must stay in one space; use `analytics::greeks::sqrt_q64_to_ui_price` (decimals-adjusted) for anything compared against `--entry-price` or displayed.
- **Q64.64 precision.** sqrt_price is Q64.64 (`u128`); never cast a large `u128` straight to `f64` — split at the 64-bit point and cast halves (see `sqrt_price.rs`).
- **Shadow vs live.** `watch` defaults to *shadow* mode (decisions logged, nothing submitted). `--live` requires the DB-backed shadow gate to pass (≥14 days of zero-error data). Every potential submission must route through `ShadowGuard::submit`. See `src/execution/CLAUDE.md`.
- **Risk monitor.** `strategy::risk_monitor` enforces drawdown halt (kill-switch, survives restart until cleared via SQL), IL pause, and Drift margin checks. State is persisted to Postgres; session-volatile fields (peak_pnl) reset on start, `halt_flag` does not.
- **Async hygiene.** The WS callback must stay cheap — tick notifications flow through a bounded mpsc channel to a processor task; blocking RPC inside async uses `block_in_place`. Keypairs/secrets/RPC keys come only from env vars (never files/literals) and must never be logged (redact URLs).
- **Fees.** One-shot `position` reports only on-chain `fee_owed_*` (no session baseline). `watch` records `fee_growth_global_*` baselines at start and accrues from the delta — pairing `fee_growth_global` with the position's `fee_growth_inside` checkpoint is a protocol mismatch ("Bug 2").
- Test math with `proptest` invariants (amounts ≥ 0, IL ≤ 0, delta < 0 / gamma > 0 strictly in-range and 0 outside, no NaN/Inf on degenerate inputs) and exact-value golden vectors.

## Math Reference

CLMM position amounts given liquidity `L`, price `P`, range `[Pa, Pb]`:
- `P < Pa`: `x = L*(1/√Pa - 1/√Pb)`, `y = 0`
- `P > Pb`: `x = 0`, `y = L*(√Pb - √Pa)`
- `Pa ≤ P ≤ Pb`: `x = L*(1/√P - 1/√Pb)`, `y = L*(√P - √Pa)`

LP delta (when in range): `delta = -L / (2√P * P)` — negative means naturally short volatility (source of IL).

Real P&L = `fees_earned - impermanent_loss`

## Dependencies

- `solana-client` 4.0-beta, `solana-sdk` 4 (façade over component crates); `orca_whirlpools_core` 2 for tick↔sqrt-price math. No `anchor-client` — accounts are parsed directly with `borsh` (discriminator skipped, owner verified).
- `tokio` (full), `tokio-tungstenite` (native-tls) + `futures-util`, `reqwest` (GeckoTerminal OHLCV), `sqlx-core`/`sqlx-postgres` 0.8 (Postgres + TimescaleDB), `clap` v4 (derive + env), `toml` (research config).
- `anyhow`, `thiserror`, `tracing` + `tracing-subscriber`, `serde`/`serde_json`, `base64`, `chrono`.
- `binance-sdk` 45 (spot only) and a hand-rolled Coinbase WS for the CEX price feeds (no Pyth); `teloxide` 0.13 + `dptree` for the Telegram bot.
- `metrics` 0.24 + `metrics-exporter-prometheus` 0.16 for observability.
- Dev: `proptest`.

## CI & local infra

- `.github/workflows/ci.yml`: three jobs on push/PR to `master` — **lint** (`cargo fmt --check` + `cargo clippy --all-targets --all-features -D warnings`), **test** (`cargo test --all-features`; DB integration tests are `#[ignore]`'d), **security** (`cargo audit`). `RUSTFLAGS: -D warnings` is set globally.
- `.github/workflows/claude-review.yml`: the automated PR review governed by the "Code Review Guidelines" below.
- `docker-compose.yml` + `Makefile` bring up Postgres/TimescaleDB bound to `127.0.0.1`; `POSTGRES_PASSWORD` must match `DATABASE_URL`. Schema is embedded (`storage::run_migrations`) — there is no `migrations/` dir; `db migrate` is re-runnable.

## Configuration (env vars)

`SOLANA_RPC_URL`, `DATABASE_URL`, `RPC_TIMEOUT_SECS` (global flags too); `LP_INSPECTOR_KEYPAIR` (base58, required by `rebalance`/`hedge`); `TELEGRAM_BOT_TOKEN` / `TELEGRAM_CHAT_ID` / `TELEGRAM_ALLOWED_USER_IDS` (for `watch --telegram`); `METRICS_LISTEN` (pull) or `METRICS_PUSH_URL` + `METRICS_PUSH_INTERVAL_SECS` (push); `COINBASE_SYMBOL`. See `.env.example` and the README table. Secrets are env-only — never config files or code.

## Code Review Guidelines

These instructions govern the automated PR review (`.github/workflows/claude-review.yml`).

**What to look for.** For this codebase, weigh — in priority order:

1. **Money-handling correctness** — CLMM math errors, liquidity/amount over/underflow, sign errors in IL/delta, off-by-one in tick math, rounding that leaks value. Cross-check math against the formulas in the "Math Reference" section above and against the Orca Whirlpool JS SDK.
2. **Security** — keypair or secret leakage, unverified program ownership before account deserialization, unchecked RPC/feed data, missing slippage/approval guards in execution paths.
3. **Robustness** — `unwrap()`/`expect()`/`panic!` in production paths, swallowed errors, missing reconnect on WebSocket feeds.
4. **Performance** — blocking calls in async paths, unbounded retries, N+1 RPC calls, missing connection pooling.

**Severity.** Tag every finding with one of:
- 🔴 **Blocker** — correctness / security / money-loss / panic in a production path. Must be fixed before merge.
- 🟡 **Should-fix** — a real issue that is not merge-blocking.

Anything that would be a style/formatting nit: do not post it. `cargo fmt` and `cargo clippy -D warnings` already gate those in CI.

**Depth (the review "level").** Default is **BALANCED**: report real issues of medium-or-higher confidence; skip speculative concerns. To change rigor, edit the `Depth:` line in the workflow prompt:
- *Blockers-only* — post only 🔴 findings; fewest comments.
- *Balanced* (default) — 🔴 + 🟡, medium+ confidence.
- *Exhaustive* — broad coverage including lower-confidence/edge findings; more comments, more false positives.

**Path-scoped rules.** The money paths carry stricter, code-specific rules in their own files — apply them on top of the above when a PR touches them:
- `src/math/CLAUDE.md` — purity, Q64.64 precision, unit-space (BUG-qr9), math invariants.
- `src/execution/CLAUDE.md` — ShadowGuard chokepoint, phased rollout, secrets, rebalance/tick safety.

**Style.** Be concise and actionable — state the problem, why it matters, and the fix. Post inline comments on the exact lines; use the single summary comment for a severity tally and cross-cutting notes. If nothing is worth raising, say so briefly — do not manufacture findings.
