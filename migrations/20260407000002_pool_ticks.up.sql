-- Pool snapshots. TimescaleDB hypertable partitioned on ts.
-- Stores sqrt_price as NUMERIC to fit Q64.64 (up to ~39 digits).
CREATE EXTENSION IF NOT EXISTS timescaledb;

CREATE TABLE pool_ticks (
    pool       TEXT          NOT NULL,
    ts         TIMESTAMPTZ   NOT NULL,
    sqrt_price NUMERIC(80,0) NOT NULL,
    tick       INTEGER       NOT NULL,
    liquidity  NUMERIC(80,0) NOT NULL,
    PRIMARY KEY (pool, ts)
);

SELECT create_hypertable('pool_ticks', 'ts', if_not_exists => TRUE);

CREATE INDEX pool_ticks_pool_ts_idx ON pool_ticks (pool, ts DESC);