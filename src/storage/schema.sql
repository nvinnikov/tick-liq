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

-- TimescaleDB hypertable for per-tick liquidity snapshots
CREATE TABLE IF NOT EXISTS pool_ticks (
    time TIMESTAMPTZ NOT NULL,
    pool_address TEXT NOT NULL,
    tick_index INT NOT NULL,
    liquidity_net BIGINT NOT NULL
);
-- SELECT create_hypertable('pool_ticks', 'time', if_not_exists => TRUE);

-- P&L history per position
CREATE TABLE IF NOT EXISTS pnl_history (
    time TIMESTAMPTZ NOT NULL,
    mint TEXT NOT NULL,
    fees_usd DOUBLE PRECISION NOT NULL,
    il_usd DOUBLE PRECISION NOT NULL,
    net_usd DOUBLE PRECISION NOT NULL,
    price DOUBLE PRECISION NOT NULL
);
-- SELECT create_hypertable('pnl_history', 'time', if_not_exists => TRUE);
