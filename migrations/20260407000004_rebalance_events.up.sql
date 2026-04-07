-- Audit log of rebalance actions taken on a position.
CREATE TABLE rebalance_events (
    id          BIGSERIAL   PRIMARY KEY,
    position_id BIGINT      NOT NULL REFERENCES positions(id) ON DELETE CASCADE,
    ts          TIMESTAMPTZ NOT NULL DEFAULT now(),
    old_range   INT4RANGE   NOT NULL,
    new_range   INT4RANGE   NOT NULL,
    reason      TEXT        NOT NULL,
    tx_sig      TEXT        NOT NULL
);

CREATE INDEX rebalance_events_position_ts_idx ON rebalance_events (position_id, ts DESC);
CREATE INDEX rebalance_events_tx_sig_idx       ON rebalance_events (tx_sig);
