-- Tracked LP positions across CLMM pools (Orca Whirlpool, Raydium CLMM, ...).
CREATE TABLE positions (
    id          BIGSERIAL PRIMARY KEY,
    mint        TEXT        NOT NULL UNIQUE,
    pool        TEXT        NOT NULL,
    owner       TEXT        NOT NULL,
    lower_tick  INTEGER     NOT NULL,
    upper_tick  INTEGER     NOT NULL,
    opened_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at   TIMESTAMPTZ,
    CHECK (lower_tick < upper_tick)
);

CREATE INDEX positions_pool_idx  ON positions (pool);
CREATE INDEX positions_owner_idx ON positions (owner);
CREATE INDEX positions_open_idx  ON positions (closed_at) WHERE closed_at IS NULL;
