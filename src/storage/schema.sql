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
