-- Per-position P&L samples. TimescaleDB hypertable partitioned on ts.
CREATE TABLE pnl_history (
    position_id BIGINT        NOT NULL REFERENCES positions(id) ON DELETE CASCADE,
    ts          TIMESTAMPTZ   NOT NULL,
    fees_x      NUMERIC(80,0) NOT NULL,
    fees_y      NUMERIC(80,0) NOT NULL,
    il_usd      DOUBLE PRECISION NOT NULL,
    net_usd     DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (position_id, ts)
);

SELECT create_hypertable('pnl_history', 'ts', if_not_exists => TRUE);

CREATE INDEX pnl_history_position_ts_idx ON pnl_history (position_id, ts DESC);
