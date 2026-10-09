-- Fetched IDLs, so a restart doesn't refetch every program. Errors are not
-- persisted: they are retried within minutes anyway.
CREATE TABLE idl_cache (
    program_id   TEXT PRIMARY KEY,
    fetch_status TEXT        NOT NULL CHECK (fetch_status IN ('found', 'missing')),
    location     TEXT CHECK (location IN ('legacy', 'program_metadata')),
    idl_format   TEXT,
    idl_json     JSONB,
    fetched_at   TIMESTAMPTZ NOT NULL
);
