# HTTP API

`failscope serve` (default `127.0.0.1:8080`, `BIND` to change). JSON only.

## Conventions

- **Window:** `hours` = last N hours, default 24, max 720. Measured on `block_time`, or on ingest time for rows whose block time hasn't been backfilled yet.
- **Paging:** `limit` (default 50, max 500) and `offset` (max 1,000,000).
- **List responses:** `{"data": [...], "page": {"limit", "offset"}, "window_hours"}`.
- **Times:** RFC 3339 strings in UTC (`2026-10-09T08:51:01Z`).
- **Errors:** `{"error": "..."}` with 400 (bad parameter), 404 (unknown signature) or 500. Database details are logged, never returned.

## Endpoints

| Method & path | Parameters | Returns |
|---|---|---|
| `GET /health` | | `{"status": "ok"}` |
| `GET /api/programs/top` | `hours`, `limit`, `offset` | Programs by failure count: `program_id`, `failures`, `distinct_errors`, `last_seen`. Tx-level errors (no program) are excluded. |
| `GET /api/programs/{program_id}/errors` | `hours`, `limit`, `offset` | One program's errors by count: `error_kind`, `error_code`, `error_name`, `decode_source`, `failures`, `last_seen`. |
| `GET /api/failures` | `hours`, `limit`, `offset`, `program`, `error_name` | Failed transactions, newest slot first. Each row has the full decoded record (see below). |
| `GET /api/failures/{signature}` | | One decoded failure, or 404. |
| `GET /api/failures/timeseries` | `hours`, `bucket` = `minute`\|`hour`\|`day` (default `hour`; `minute` needs `hours` ≤ 24), `program` | `{"data": [{"start", "failures"}], "bucket", "window_hours", "program"}`, oldest first, empty buckets as 0. |
| `GET /api/coverage` | `hours` | `{"data": [{"decode_source", "failures", "share"}], "total", "window_hours"}`: how many failures each decode source explained. |

### Failure record

`signature`, `slot`, `block_time`, `signer`, `fee`, `priority_fee`,
`cu_requested`, `cu_consumed`, `uses_alt`, `log_truncated`,
`top_level_ix_index`, `root_program_id`, `failing_program_id`, `cpi_depth`,
`attribution_confidence` (`high`|`low`), `error_kind`, `error_code`,
`error_name`, `error_message`, `error_account`, `decode_source`
(`anchor_log`|`anchor_framework`|`idl`|`native`|`runtime`|`unknown`).

## Examples

```sh
curl 'localhost:8080/api/programs/top?hours=24&limit=10'
curl 'localhost:8080/api/programs/JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4/errors'
curl 'localhost:8080/api/failures/timeseries?hours=6&bucket=minute'
curl 'localhost:8080/api/failures?error_name=SlippageToleranceExceeded&limit=20'
```
