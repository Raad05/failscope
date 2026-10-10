-- Alert state changes, one row per transition. A program's current state is
-- its latest row. Rows are written only after the webhook accepted them.
CREATE TABLE alert_events (
    id                  BIGSERIAL PRIMARY KEY,
    program_id          TEXT             NOT NULL,
    state               TEXT             NOT NULL CHECK (state IN ('firing', 'resolved')),
    at                  TIMESTAMPTZ      NOT NULL DEFAULT now(),
    window_minutes      INTEGER          NOT NULL,
    current_count       BIGINT           NOT NULL,
    baseline_per_window DOUBLE PRECISION NOT NULL,
    threshold           DOUBLE PRECISION NOT NULL
);

CREATE INDEX alert_events_program_at ON alert_events (program_id, at DESC);
