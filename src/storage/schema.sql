-- Positions opened/closed by the manager
CREATE TABLE IF NOT EXISTS positions (
    id BIGSERIAL PRIMARY KEY,
    mint TEXT NOT NULL UNIQUE,
    protocol TEXT NOT NULL,        -- 'orca' | 'raydium'
    pool_address TEXT NOT NULL,
    tick_lower INT NOT NULL,
    tick_upper INT NOT NULL,
    entry_price DOUBLE PRECISION,
    opened_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    closed_at TIMESTAMPTZ
);

-- TimescaleDB hypertable for per-tick liquidity snapshots.
-- UNIQUE (pool_address, slot) makes writes idempotent across WS reconnects (PERSIST-04).
CREATE TABLE IF NOT EXISTS pool_ticks (
    time TIMESTAMPTZ NOT NULL,
    pool_address TEXT NOT NULL,
    slot BIGINT NOT NULL,
    tick_current INT NOT NULL,
    sqrt_price NUMERIC(80,0) NOT NULL,
    liquidity NUMERIC(80,0) NOT NULL,
    fee_growth_global_a NUMERIC(80,0) NOT NULL,
    fee_growth_global_b NUMERIC(80,0) NOT NULL,
    UNIQUE (pool_address, slot)
);
-- SELECT create_hypertable('pool_ticks', 'time', if_not_exists => TRUE);

-- P&L history per position (PERSIST-02 fields).
-- pool_address added for join queries without going through positions table.
CREATE TABLE IF NOT EXISTS pnl_history (
    time TIMESTAMPTZ NOT NULL,
    mint TEXT NOT NULL,
    pool_address TEXT NOT NULL,
    fees_earned DOUBLE PRECISION NOT NULL,
    il_usd DOUBLE PRECISION NOT NULL,
    net_pnl DOUBLE PRECISION NOT NULL,
    position_value DOUBLE PRECISION NOT NULL,
    price DOUBLE PRECISION NOT NULL
);
-- SELECT create_hypertable('pnl_history', 'time', if_not_exists => TRUE);

-- Shadow rebalance decisions (SHADOW-02): one row per rebalance trigger in shadow mode.
-- Server-side DEFAULT NOW() timestamp; BIGSERIAL id — no client-supplied id (T-02-04).
CREATE TABLE IF NOT EXISTS shadow_rebalances (
    id                      BIGSERIAL PRIMARY KEY,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    pool_address            TEXT NOT NULL,
    trigger_reason          TEXT NOT NULL,
    price                   DOUBLE PRECISION NOT NULL,
    simulated_range_width   DOUBLE PRECISION,
    simulated_fees_earned   DOUBLE PRECISION,
    simulated_il_usd        DOUBLE PRECISION,
    simulated_net_pnl       DOUBLE PRECISION,
    error_flag              BOOLEAN NOT NULL DEFAULT FALSE,
    error_message           TEXT
);

CREATE INDEX IF NOT EXISTS idx_shadow_rebalances_pool_created
    ON shadow_rebalances (pool_address, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_shadow_rebalances_pool_error
    ON shadow_rebalances (pool_address) WHERE error_flag = true;

-- Risk state persistence (RISK-04): one row per pool_address, upserted on every tick.
-- halt_flag survives restart -- operator must manually clear via SQL (D-12).
CREATE TABLE IF NOT EXISTS risk_state (
    pool_address          TEXT PRIMARY KEY,
    peak_pnl              DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    current_drawdown_pct  DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    pause_flag            BOOLEAN NOT NULL DEFAULT FALSE,
    halt_flag             BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Telegram bot operator pause column (TG-01 / D-04): added idempotently at startup.
-- operator_pause is independent from IL-triggered pause_flag (different lifecycle).
ALTER TABLE risk_state ADD COLUMN IF NOT EXISTS operator_pause BOOLEAN NOT NULL DEFAULT FALSE;
