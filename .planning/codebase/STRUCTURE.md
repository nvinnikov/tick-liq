# STRUCTURE.md — Directory Structure

## Root Layout

```
tick-liq/
├── Cargo.toml              # workspace / package manifest
├── Cargo.lock
├── CLAUDE.md               # project spec + AI guidance
├── config/
│   └── config.toml.example # config template (never commit real config)
├── src/
│   ├── main.rs             # CLI entrypoint (clap subcommands)
│   ├── lib.rs              # library root (re-exports for tests)
│   ├── config.rs           # Config struct, TOML + env deserialization
│   ├── math/
│   │   ├── mod.rs
│   │   ├── clmm.rs         # tick↔price, liquidity→amounts
│   │   ├── il.rs           # impermanent loss formulas
│   │   └── greeks.rs       # delta, gamma
│   ├── data/
│   │   ├── mod.rs
│   │   ├── rpc.rs          # Solana RPC connection pool
│   │   ├── pool.rs         # Whirlpool / CLMM account structs
│   │   └── price_feed.rs   # Pyth + CEX price streams
│   ├── strategy/
│   │   ├── mod.rs
│   │   ├── monitor.rs      # position state monitoring loop
│   │   ├── rebalance.rs    # rebalance signal logic
│   │   └── range_optimizer.rs
│   ├── execution/
│   │   ├── mod.rs
│   │   ├── rebalance_exec.rs
│   │   └── hedge.rs        # Drift Protocol perp hedging
│   └── storage/
│       ├── mod.rs
│       └── db.rs           # sqlx PgPool, write positions/events
└── tests/
    ├── math_tests.rs       # property-based tests (proptest)
    └── strategy_tests.rs
```

## Key Locations

| What | Where |
|------|-------|
| CLI subcommands | `src/main.rs` |
| CLMM price/liquidity math | `src/math/clmm.rs` |
| IL calculation | `src/math/il.rs` |
| Pool account parsing | `src/data/pool.rs` |
| Rebalance decision | `src/strategy/rebalance.rs` |
| Transaction building | `src/execution/rebalance_exec.rs` |
| DB writes | `src/storage/db.rs` |
| Config struct | `src/config.rs` |

## Naming Conventions

- Files and modules: `snake_case`
- Types, traits, enums: `PascalCase`
- Functions and variables: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`
- Test functions: `snake_case` prefixed with `test_` (unit) or in `tests/` (integration)

## Where to Start

New contributors: begin with `src/math/` — no external dependencies, fully self-contained, testable without any network or DB setup.