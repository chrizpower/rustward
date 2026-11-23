# Operational contracts

Scheduling, explicit reload, age-based retention, and offline backup and
restore are implemented. Telemetry is accepted future work. See the [service interface](service.md).

## Scheduling

The service evaluates the accepted five-field UTC cron subset with `croner`.
Each schedule ID is its path relative to the schedule root without `.toml`.
SQLite schema 8 stores occurrence decisions, schedule revisions, evaluation
checkpoints, gaps, and pause state. A decision, its admitted run, and its
checkpoint commit in one transaction. Manual requests and scheduled
occurrences share the queue and retain their own identities.

Occurrence identity is the schedule ID and UTC due time. Edits keep the ID and
cannot reopen a handled due time. A rename uses the new path as a new ID.
Existing runs and earlier history remain. A new schedule starts unpaused
subject to the global schedule pause. Its first checkpoint covers the current
UTC minute, with an unknown preceding interval. On reactivation of an old ID
its checkpoint remains protected and its individual pause resets.

The service samples wall time and monotonic time. It evaluates the next UTC
minute when coverage is continuous. On restart, a gap covers any interval
after the stored checkpoint through the current minute. If more than one
minute elapses without evaluation or the wall clock jumps, the service records
a gap and skips the affected work. A backward jump cannot lower a checkpoint.
The service treats a backward wall-clock step or a difference over two seconds
between wall and monotonic elapsed time as a clock change. It counts missed
occurrences for intervals up to 24 hours when the schedule was continuously
configured with the same revision and the time bounds are known.

Schedule pause records known due occurrences as skipped. A paused schedule
does not affect manual admission or dispatch. An active run can have one queued
successor from the same schedule. Further occurrences skip while that successor
remains queued. The same queue limit applies to both kinds of admission.

## Live reload

Send `{"command":"reload"}` through `rustward ctl`. The service reads the same
configuration path it used at startup and validates the entire candidate.
Storage, limit, and retention setting changes require a restart. The service
keeps the previous configuration if loading, validation, or activation fails.
Accepted runs and handled occurrences are retained. Activation applies the new
definitions to future minutes.

Publish a complete immutable configuration directory at the configured path.
The loader holds one directory descriptor for the service file and definition
roots that live within that directory. Keep externally located definition roots
unchanged throughout a reload. Keep the previous directory available until any
reload that opened it has finished. The reader rejects observed changes but
does not provide a filesystem snapshot of in-place edits.

## Private web access

Append `--web /run/rustward/web.sock https://rustward.example 33` to
`rustward serve` to enable the console. Replace the path, HTTPS origin and
UID with the deployment values. Use a separate UID for the authenticating
Nginx worker. Give the service ownership of the socket directory and allow
the proxy to traverse it. Remove an old socket after a crash only after
confirming the previous service has stopped.

Nginx must authenticate every console route and preserve the browser's Host
header. Set the backend to the Unix socket and clear the Authorization header
before forwarding. Rustward checks the peer UID, the exact Host header, and,
for API requests, the configured Origin and `X-Rustward-CSRF: 1` header.
The web socket has no TCP listener. This enforces proxy access even when
another account can connect locally. Deployment configuration stays in
the deployment repository.

The console uses embedded assets, typed task forms, and durable manual request
keys. It polls each output stream in 16 KiB chunks and renders it as text.
The console reconnects using the run ID and limits the displayed tail per
stream. It shows a truncation or reclamation notice when stored evidence
records it. Queue, run history, and schedule results use bounded pages.

`just web-preview` uses a disposable database, synthetic tasks, and a fake
supervisor. On macOS the preview uses a Linux container and publishes its
port on host loopback. Run `just web-test` on Linux to test the embedded
production assets in headless Chromium. It tests request retries, reconnecting,
input validation, text rendering, cancellation, schedule controls, and review.
Run `just acceptance-container` on the isolated Docker host for the web case.
It checks Nginx authentication, proxy peer enforcement, origin checks, and a real
systemd run. It records `fixture/evidence/web.json`.

## Age-based retention

The service uses the recorded UTC completion time and the configured number of
24-hour days. An eligible run expires at the cutoff or earlier. Unknown or
negative completion times remain in history. A batch removes up to 32 runs.
The service checks again after one second if it removed history or the database
was busy, and after one minute if there was no eligible history.

Queued runs, active runs, unresolved reviews and their output, and permanent
admission records are preserved. Schedule occurrence decisions and review
acknowledgements also remain. An expired run no longer has a link in schedule
history. A matching manual request retry returns `history_removed` with the
original run ID. Log files are removed before history is committed. A storage
failure closes the service under [Gravity P06](../GRAVITY.md); after repair and
restart, retention can retry a file removal that already succeeded. Capacity
based log reclamation continues to run before dispatch as needed. Global
metadata bounds and monitoring of permanent-record growth remain open.

## Offline application-state backup and restore

Run these commands as the service account on Linux. Stop the Rustward service
and its local writers before backup. Backup uses the database's dispatch lock
and fails while the service owns the database. The destination must be a new
directory inside a private directory owned by the service account.

```sh
install -d -m 700 /path/to/private-state
rustward state backup /path/to/rustward.toml /path/to/private-state/backup
rustward state restore /path/to/private-state/backup /path/to/private-state/restored
rustward state reviews /path/to/private-state/restored
```

The backup contains a SQLite snapshot, a copy of retained log files, and a
manifest of file sizes and SHA-256 hashes. Backup detects incomplete retained
output with a known nonzero size for unresolved completed reviews. Restore
checks the manifest and SQLite integrity before it publishes the new directory.
Backups contain submitted values and retained task output. Store them with the
same access controls as live state. Keep definitions, prepared runtime and host
catalogs, and deployment credentials with their existing owners. Provide them
separately on the restored host.

Restore rotates the manual-key issuer and closes admission and dispatch before
the new database is published. A known old key still identifies its previous
admission. An unknown old key is rejected. The service can open the restored
database only after explicit release. It cannot adopt an old local process.
For each run listed by `state reviews`, record one decision and a short note
without credentials or sensitive output:

```sh
rustward state decide /path/to/private-state/restored RUN_ID cancel 'Checked the external effects and discarded the queued run'
rustward state release /path/to/private-state/restored
```

Use `cancel` for queued work that must be discarded. Use `keep` only after
checking that the queued run can execute safely. For an active run, use
`interrupt` only after confirming that its original local processes are gone.
This records an interruption. The run still requires the normal external
review and acknowledgement before dispatch resumes.

Disable the original instance before bringing up restored state. Point the new
configuration at the restored `state.db` and `logs`. Use a fresh work directory.
Start the service. Dispatch starts paused and requires an explicit resume.
Use the supported restore command for every backup. A raw rollback of a live
database has no restore review or new issuer generation.

Run `just recovery-test` on Linux for the synthetic rehearsal. The Docker
acceptance image build runs the same tests before running the isolated host suite.

## Telemetry

On 2026-11-08, the user accepted structured service logs and private metrics for
the monitoring service. Collector configuration belongs in `the deployment repository`.

### Service logs

Provide structured service logs for startup and shutdown, configuration
acceptance and rejection, admission, dispatch, run completion, recovery,
operational controls, and state or task-log storage failures. Report only
implemented and observed events. A launch request is not proof of process start.
A successful admission log must follow the durable admission decision.

Records need a timestamp, severity, stable event name, component, and bounded
reason or result fields. Where relevant, include task, schedule, and run IDs as log fields to correlate
with retained state.
Use established run phases and results from the [glossary](../GLOSSARY.md).

Keep task stdout/stderr in the existing [capture interface](service.md#bounded-output-capture). Do not
copy task output, submitted input values, command arguments, credentials,
request keys, or secret-bearing error details into service logs. Apply the same
restrictions at debug levels. Use bounded reason codes instead of raw error
strings where errors can contain payloads. Logs support diagnosis. They do not
replace durable admission records, run history, or acknowledgement evidence.

### Metrics collection

Provide a private, read-only HTTP `GET /metrics` endpoint with a
Prometheus-compatible exposition format and matching content type. the monitoring service must be
able to scrape it without a browser session. A scrape must not admit work,
change controls, or execute a task. Keep metrics available during admission or
dispatch blocks where the service can still serve requests, so the block remains
observable.

Keep access private and restrict it to authorized monitoring access. Specify the
listener, access mechanism, and collector route before deployment. Do not infer
the monitoring service's address, credentials, collector software, or network access from its name.
Service telemetry settings belong in `rustward.toml`. No telemetry fields are
implemented in the current schema. Exact keys and defaults remain open.

Expose these measurements as their owning components are implemented:

| Measurement | Required meaning |
| --- | --- |
| Queue depth | Current accepted runs waiting for dispatch |
| Active runs | Current runs holding an execution slot, including launch and cleanup |
| Admissions and rejections | New durable admissions and rejected submissions, with bounded trigger/reason categories. A matching retry is not a new admission |
| Completed runs | Recorded completions by result. Cancellation before dispatch is included |
| Run duration | A defined duration distribution. Specify start/end points and treatment of queued cancellation before implementation |
| Skipped occurrences | Known skipped schedule occurrences by bounded reason. Unknown schedule gaps must not be converted to invented counts |
| Dispatch blocks | Current block state by bounded reason, including pause, external uncertainty, and protected log capacity |
| Log truncation | Runs whose task output reached its retention limit, distinct from storage failures |
| Storage failures | Observed state or task-log storage failures by bounded component/category |

Use bounded label sets. Do not use run IDs, task IDs, schedule IDs, input values,
paths, request keys, arbitrary error text, or definition revisions as metric
labels. This prevents file changes and individual requests from creating an
unbounded number of time series. Detailed per-run diagnosis belongs in logs
and retained state.

Specify metric names, types, units, label values, duration boundaries, histogram
buckets, and reset behavior before implementation. Document whether each count
covers the current process lifetime or retained history. Do not count a retried
submission as another run or count the same committed transition twice during
recovery. Metrics are operational observations, not an admission ledger. Do not
publish fabricated zero values for components that are not implemented or whose
state is unknown.

### Bounds and failure behavior

Bound log record size, log buffering, metric series, scrape work, and concurrent
scrapes. Specify numeric bounds and overload handling before implementation.
Telemetry must not introduce an unbounded queue or hold an admission transaction
open while waiting for a remote collector. A failed scrape or an unreachable the monitoring service
does not change a run result or prove that execution failed.

Existing P06 behavior for state and retained task-log storage failures remains
mandatory. The response to service-log sink failure, its relation to P06, and
how to expose telemetry loss remain open decisions. Do not silently weaken P06
or equate a remote collection outage with a local state-storage failure.
Service-log transport, encoding, rotation, and retention also need specification.
Per-run task-log limits do not define service-log retention.

### Verification and implementation gates

- Check structured records for each implemented event and valid correlation IDs.
  Use synthetic sensitive values to prove exclusion at every enabled log level.
- Parse endpoint output with a Prometheus-compatible parser and verify content
  type, types, labels, units, and escaping. Test private access and read-only use.
- Verify counters against durable decisions, matching retries, queued
  cancellation, and recovery. Test queue/active gauges and dispatch blocks.
- Flood logs and issue concurrent scrapes. Verify the selected memory, record,
  series, and request bounds, including slow or failed sinks.
- Check truncation separately from storage failure. Verify unknown state and
  schedule gaps are not reported as successful or zero-failure operation.
- In separate deployment work, prove an end-to-end scrape from the monitoring service and service
  log collection through the selected transport. Repository tests alone do not
  prove collector connectivity or production collection.

Health and readiness mechanisms remain separate F15 work. Select libraries and
settle the open interface and failure decisions before implementation. This
contract does not require distributed tracing or a new log storage service.
