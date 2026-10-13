# Configuration

`failscope` is one binary with three subcommands. Every option is a flag and
an environment variable; `.env` in the working directory is read if present
(`cp .env.example .env`). `failscope <command> --help` lists the same options.
Values marked **secret** are hidden in `--help` and never logged.

## `failscope ingest`

Streams failed transactions, decodes and scores them, writes Postgres.

| Variable | Flag | Default | Meaning |
|---|---|---|---|
| `YELLOWSTONE_ENDPOINT` | `--endpoint` | required | Yellowstone gRPC URL. `http://` for the local plugin, `https://` (TLS, system root certificates) for hosted providers. |
| `YELLOWSTONE_X_TOKEN` | `--x-token` | none | **Secret.** Sent as the `x-token` header. Empty means none. |
| `DATABASE_URL` | `--database-url` | required | **Secret.** Postgres URL. Migrations run on start. |
| `RPC_URL` | `--rpc-url` | required | JSON-RPC endpoint, used only to fetch program IDLs. May carry an API key in the URL; treat it as secret then. |
| `INGEST_STREAM` | `--stream` | `default` | Name of this stream's cursor. Use a new name when pointing an existing database at a different cluster, or ingest resumes from the other cluster's slot (see [operations](operations.md#troubleshooting)). |
| `SCORE_CONFIG` | `--score-config` | built-in | Path to a JSON file overriding bot-scoring settings (below). |

## `failscope serve`

JSON API ([api.md](api.md)) and the dashboard at `/`.

| Variable | Flag | Default | Meaning |
|---|---|---|---|
| `DATABASE_URL` | `--database-url` | required | **Secret.** |
| `BIND` | `--bind` | `127.0.0.1:8080` | Listen address. The API has no authentication; see [operations](operations.md#security) before binding anything but loopback. |

## `failscope alert`

Evaluates per-program spikes every interval and POSTs a webhook on state changes (D23).

| Variable | Flag | Default | Meaning |
|---|---|---|---|
| `DATABASE_URL` | `--database-url` | required | **Secret.** |
| `ALERT_WEBHOOK_URL` | `--webhook-url` | required | **Secret.** Receives JSON with a Slack-compatible `text` field. Must not be empty. |
| `ALERT_METRIC` | `--metric` | `failures` | `failures`, or `human-signers`: distinct likely-human wallets with a failure, per minute (D27). |
| `ALERT_WINDOW_MINUTES` | `--window-minutes` | `5` | Minutes counted as "now". |
| `ALERT_BASELINE_MINUTES` | `--baseline-minutes` | `60` | Minutes before the window used as the program's normal rate. Must be ≥ the window. |
| `ALERT_FACTOR` | `--factor` | `3` | Fire when the window count reaches this multiple of the baseline. ≥ 1. |
| `ALERT_MIN_FAILURES` | `--min-failures` | `10` | Never fire below this count, whatever the baseline. |
| `ALERT_INTERVAL_SECS` | `--interval-secs` | `60` | Seconds between evaluations. ≥ 5. |
| `ALERT_PROGRAMS` | `--program` | all | Program ids to watch, comma-separated or repeated. |
| `DASHBOARD_URL` | `--dashboard-url` | none | Base URL linked from alert messages. |

The threshold is `max(min_failures, factor × max(baseline per window, 1))`; a
firing alert resolves below 75% of it.

## All commands

| Variable | Default | Meaning |
|---|---|---|
| `RUST_LOG` | `info,sqlx=warn` | Log filter ([`tracing` env-filter syntax](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)). `failscope=debug` logs every stored failure with its bot class. |

Logs are plain text when not written to a terminal.

## Bot-scoring settings (`SCORE_CONFIG`)

A JSON object; every field is optional and unknown fields are rejected, so a
typo fails at startup instead of being ignored. Defaults (D27):

```json
{
  "window_slots": 1500,
  "high_rate_failures": 20,
  "very_high_rate_failures": 100,
  "retry_run_min": 5,
  "retry_gap_slots": 3,
  "fee_outlier_percentile": 0.95,
  "fee_min_samples": 50,
  "fee_sample_size": 1000,
  "low_activity_max_failures": 3,
  "bot_threshold": 60,
  "human_below": 30,
  "weights": {
    "high_rate": 35,
    "very_high_rate": 20,
    "retry_loop": 25,
    "jito_tip": 20,
    "fee_outlier": 10,
    "silent_program": 10,
    "low_activity": -20
  }
}
```

| Field | Meaning |
|---|---|
| `window_slots` | Sliding window per wallet, in slots (1500 ≈ 10 minutes). |
| `high_rate_failures`, `very_high_rate_failures` | Failures in the window that add `weights.high_rate`, then also `weights.very_high_rate`. |
| `retry_run_min`, `retry_gap_slots` | Same program and error at least this many times, at most this many slots apart: `weights.retry_loop`. |
| `fee_outlier_percentile`, `fee_min_samples`, `fee_sample_size` | Compute-unit price at or above this share of the program's last `fee_sample_size` prices (once it has `fee_min_samples`): `weights.fee_outlier`. |
| `low_activity_max_failures` | At most this many failures, no Jito tip, no fee outlier: `weights.low_activity` (negative). |
| `bot_threshold`, `human_below` | Score ≥ `bot_threshold` is `likely_bot`; below `human_below` is `likely_human`; between is `unclear`. |

Scores are computed when a failure is ingested and stored with it. Changing
the settings affects new failures only.

## Docker Compose

`docker-compose.yml` sets `DATABASE_URL` and `BIND` itself and passes the
rest through from `.env` or the shell. Endpoints for the ingest container
use separate names, so one `.env` serves both `cargo run` and compose:

| Variable | Default | Meaning |
|---|---|---|
| `COMPOSE_YELLOWSTONE_ENDPOINT` | `http://host.docker.internal:10000` | Ingest's gRPC endpoint inside the container. Set it to the provider URL for devnet. |
| `COMPOSE_RPC_URL` | `http://host.docker.internal:8899` | Ingest's JSON-RPC URL inside the container. |

Profiles: none = Postgres only; `app` = ingest + serve; `alert` = alert. On
plain Docker Engine (not Desktop), add `-f docker-compose.engine.yml` so
ingest can reach a validator on the host's loopback (D26). To use
`SCORE_CONFIG` in Docker, mount the file into the ingest container and point
the variable at the mounted path.
