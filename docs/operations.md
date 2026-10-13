# Operations

How to run failscope beyond a laptop demo: what it needs, what to watch, and
what breaks. Settings are in [configuration.md](configuration.md).

## What runs

Three long-lived processes share one Postgres. Each can be restarted on its
own; all stop cleanly on SIGINT/SIGTERM.

| Process | Needs | Restart behaviour |
|---|---|---|
| `ingest` | Yellowstone gRPC, JSON-RPC (IDLs), Postgres | Resumes from its slot cursor via `from_slot`. Rebuilds IDL cache and bot-scoring windows from Postgres. Slots the provider can no longer replay are recorded in `ingest_gaps`. |
| `serve` | Postgres | Stateless. |
| `alert` | Postgres, webhook | Alert state lives in `alert_events`; a state change is recorded only after the webhook accepted it, so nothing is lost or repeated across restarts. |

Run exactly **one** `ingest` per stream name. Two processes with the same
`INGEST_STREAM` would race on the cursor.

## Security

- **The API and dashboard have no authentication.** Everything stored is
  public on-chain data, but the API can be queried heavily. `serve` binds
  `127.0.0.1` by default and compose publishes it on `127.0.0.1:8080`. To
  share it, put it behind a reverse proxy with authentication and rate
  limiting; don't bind it to a public interface as is.
- **Secrets:** `YELLOWSTONE_X_TOKEN`, `ALERT_WEBHOOK_URL`, `DATABASE_URL`,
  and `RPC_URL` when it carries an API key. Keep them in `.env` (gitignored,
  `chmod 600`) or your secret manager. They are hidden from `--help`; the
  webhook URL is stripped from delivery errors before logging.
- **The dashboard** renders on-chain strings (program ids, IDL error names)
  with `textContent` only and is served with a strict Content-Security-Policy
  (no inline scripts or styles).
- **The local validator's gRPC port** listens on `127.0.0.1` on purpose: it
  has no authentication (D26).
- The Docker image runs as an unprivileged user.

## Storage and retention

Each failure is about **1 KB** in Postgres, indexes included (measured:
18,666 rows, 19 MB). Devnet produced 3–6.5 failures per second in testing,
roughly 260–560 MB per day. Mainnet produces far more.

**There is no automatic retention yet.** Delete old rows on a schedule (cron,
or a systemd timer), keeping at least what the dashboard and alerts read
(7 days for the longest dashboard window; alerts need ~65 minutes):

```sh
docker compose exec -T db psql -U failscope -c \
  "DELETE FROM failed_tx WHERE block_time < now() - interval '7 days';"
docker compose exec -T db psql -U failscope -c "VACUUM (ANALYZE) failed_tx;"
```

`ingest_gaps`, `alert_events`, `idl_cache` and `ingest_cursor` stay small.

## Backups and upgrades

```sh
# Backup
docker compose exec -T db pg_dump -U failscope -Fc failscope > failscope.dump
# Restore into an empty database
docker compose exec -T db pg_restore -U failscope -d failscope --clean < failscope.dump
```

To upgrade: back up, pull the new code, then
`docker compose --profile app up -d --build` (add `--profile alert` if used).
Migrations (`crates/store/migrations`) run automatically when each process
starts; they are forward-only. New columns added by a migration are `NULL`
for older rows (for example, failures stored before M9 show as "not scored").

## Monitoring

- **Ingest progress:** every 2 seconds ingest logs
  `progress cursor=Some(<slot>) stats=… signers=<n>`. A cursor that stops
  moving means the stream is stuck or disconnected; reconnect attempts log
  `stream failed error=…` with backoff up to 30 s.
- **Lag:** compare the stored cursor with the cluster's slot:
  ```sh
  docker compose exec -T db psql -U failscope -c "SELECT * FROM ingest_cursor;"
  ```
- **Missed data:** `SELECT * FROM ingest_gaps ORDER BY detected_at DESC;`
  Each row is a slot range ingest could not replay after downtime.
- **Health:** `GET /health` answers `{"status":"ok"}` when `serve` is up. It
  does not check Postgres or ingest; use the progress log and the cursor for
  those.
- **Alerts:** `GET /api/alerts` lists recorded state changes.

## Running on mainnet (planned)

Development and verification so far are on devnet (D24). Before mainnet:

- Expect a much higher failure rate. Ingest inserts one row per failure from
  a single consumer; measure throughput against your Postgres and add
  batching if the stream backs up (the bounded channel will slow the
  subscription, and the provider may drop a slow client; ingest then resumes
  from the cursor).
- Plan retention and disk accordingly (see above).
- The bot-scoring thresholds were set by hand and observed on devnet only;
  calibrate them on a hand-labelled sample of mainnet wallets (D27).
- Yellowstone access on mainnet is a paid provider plan.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `stream failed error=connect: … dns error …` | The endpoint's host name doesn't resolve. Check `YELLOWSTONE_ENDPOINT` / `COMPOSE_YELLOWSTONE_ENDPOINT`. |
| `stream failed …` mentioning TLS or certificates | `https://` endpoint with no usable root certificates. The Docker image includes them; on a host, install `ca-certificates`. |
| The stream connects, then the provider rejects it (unauthenticated / permission denied) | Missing or wrong `YELLOWSTONE_X_TOKEN`, or the plan doesn't include gRPC for that cluster. |
| On plain Docker Engine, ingest can't reach the local validator | The plugin listens on `127.0.0.1` only. Use `-f docker-compose.yml -f docker-compose.engine.yml` (host networking). Docker Desktop doesn't need it. |
| A huge gap is recorded right after pointing ingest at a different cluster | The stream reused a cursor from the other cluster. Use a new `INGEST_STREAM` name (e.g. `devnet`), and delete the bogus `ingest_gaps` row. |
| Dashboard shows no data | Check the time window and that ingest is logging `progress` with `inserted` rising. Devnet always has failures; a local validator only has what you send. |
| Many wallets show "not scored" | They failed before scoring existed (M9). They age out of the dashboard windows. |
| `alert` exits with `ALERT_WEBHOOK_URL / --webhook-url is empty` | Set the webhook URL, or don't start the `alert` profile. |
| An alert never fires | The count must reach `ALERT_MIN_FAILURES` as well as `factor × baseline`. Check `ALERT_PROGRAMS`, and with `ALERT_METRIC=human-signers` that failures are being scored. |
| `send_failures` against the local validator hangs or says the program isn't deployed | The validator is still starting (wait for slot ≥ 5) or a second `dev/localnet.sh` was started on the same ledger. Stop all validators and start one. |
