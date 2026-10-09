-- One row per failed transaction. Signature is the key, so re-delivered
-- transactions (reconnects, from_slot overlap) are ignored on insert.
CREATE TABLE failed_tx (
    signature              TEXT PRIMARY KEY,
    slot                   BIGINT      NOT NULL,
    block_time             TIMESTAMPTZ,
    fee                    BIGINT      NOT NULL,
    priority_fee           BIGINT      NOT NULL,
    cu_requested           BIGINT      NOT NULL,
    cu_consumed            BIGINT,
    signer                 TEXT,
    uses_alt               BOOLEAN     NOT NULL,
    log_truncated          BOOLEAN     NOT NULL,
    top_level_ix_index     SMALLINT,
    root_program_id        TEXT,
    failing_program_id     TEXT,
    cpi_depth              INTEGER,
    attribution_confidence TEXT        NOT NULL CHECK (attribution_confidence IN ('high', 'low')),
    error_kind             TEXT        NOT NULL,
    error_code             BIGINT,
    error_name             TEXT,
    error_message          TEXT,
    error_account          TEXT,
    decode_source          TEXT        NOT NULL CHECK (decode_source IN
                               ('anchor_log', 'anchor_framework', 'idl', 'native', 'runtime', 'unknown')),
    raw_err                JSONB       NOT NULL,
    ingested_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX failed_tx_slot ON failed_tx (slot);
CREATE INDEX failed_tx_block_time ON failed_tx (block_time);
CREATE INDEX failed_tx_program_time ON failed_tx (failing_program_id, block_time);

-- Last slot whose failures are all stored, per ingest stream.
CREATE TABLE ingest_cursor (
    stream     TEXT PRIMARY KEY,
    slot       BIGINT      NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Slot ranges that could not be replayed after a disconnect.
CREATE TABLE ingest_gaps (
    id          BIGSERIAL PRIMARY KEY,
    stream      TEXT        NOT NULL,
    from_slot   BIGINT      NOT NULL,
    to_slot     BIGINT      NOT NULL,
    reason      TEXT        NOT NULL,
    detected_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
