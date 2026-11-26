# Operational contracts

The service implements scheduling, explicit reload, age-based retention,
offline backup and restore, and telemetry. See the [service interface](service.md).

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
Storage, limit, run resource, and retention setting changes require a restart. The service
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
Queued rows show the pinned runtime ID so the administrator can check the
selected runtime before resuming dispatch.

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
original run ID. Log files and the writable run directory are removed before
history is committed. Cleanup does not follow symbolic links or cross mounts
inside a writable run directory. An unreadable directory, mount, or excessive
nesting stops cleanup for operator inspection. A storage
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

Rustward writes one JSON record per line to standard error. For the systemd
service, route standard error to the journal. Deployment owns journal rotation,
retention, and collection. Task output stays in the existing run log files.

Records carry the UTC timestamp in milliseconds, severity, event, and component.
Run, task, and schedule IDs are included where available. Reasons use fixed
codes. A committed dispatch is a launch intent. A launch reply records what
the supervisor reported. Only an observed running transition emits
`run_observed_running`. Admission and completion records follow their database
commits. A restore note remains in the state database. Request keys, submitted
values, command arguments, task output, and raw error text are excluded from
structured service records.

The log worker accepts up to 128 waiting records of at most 4 KiB each. When
the queue is full or the worker cannot write a record, Rustward counts the loss
and continues. Shutdown waits up to 100 ms for a flush acknowledgement. A service
log loss or failed scrape does not trigger the state and task-log storage
failure policy.

### Private monitoring

Metrics require the existing private HTTPS proxy and an explicit opt-in:

```toml
[telemetry]
metrics = true
```

The default is false. Restart Rustward after changing this setting. Define an
exact location for each of `/metrics`, `/healthz`, and `/readyz` in the proxy. Authenticate a separate monitoring
identity and set `X-Rustward-Monitor: 1` on the upstream request only after
authorization. Clear that header on every other route. Preserve the configured
Host. The Rust service also checks the Unix socket peer UID. A browser session
alone cannot authorize a scrape. Production identity and route configuration for
monitoring belong in the deployment repository.

A scrape reads in-memory measurements. It does not use SQLite, enter the service
request queue, or change controls. The web listener allows up to 16 concurrent
connections across all routes and applies its existing header and connection
timeouts. The response uses Prometheus text format 0.0.4, a fixed set of series,
and no per-run labels.

| Metric | Type and meaning |
| --- | --- |
| `rustward_service_healthy` | Gauge of 1 when the service loop is responsive and healthy. |
| `rustward_service_ready` | Gauge of 1 when the service can process new work as execution slots become available. |
| `rustward_last_schedule_gap_timestamp_seconds` | UTC time when the service last recorded a schedule gap, excluding first installation. Omitted until a gap is recorded. |
| `rustward_schedule_unknown_gaps_total` | Counter of newly recorded gaps with unknown missed counts, excluding first installation. |
| `rustward_queue_depth` | Gauge of queued runs, seeded from SQLite and updated after committed transitions. Omitted when the state is unavailable. |
| `rustward_queue_depth_available` | Gauge of 1 if the queue depth is available, or 0 otherwise. |
| `rustward_active_runs` | Gauge of execution slots in use, including launch and cleanup. |
| `rustward_dispatch_state` | One gauge per state. The current state has value 1. States are idle, paused, review, execution capacity, log capacity, draining, and failed. |
| `rustward_admissions_total` | Counter by manual or schedule trigger. A matching retry is excluded. |
| `rustward_admission_rejections_total` | Counter of rejected manual submissions by reason. Unparsed HTTP requests are excluded. |
| `rustward_completions_total` | Counter by durable run result, including queued cancellation. |
| `rustward_run_duration_seconds` | Histogram from recorded dispatch through durable completion. Queued cancellations and missing or invalid elapsed times are excluded. Upper bounds in seconds are 0.1, 0.5, 1, 5, 30, 60, 300, and 3600, plus infinity. |
| `rustward_schedule_skips_total` | Counter of committed skips and exact missed counts in schedule gaps. Gaps with unknown counts appear in service logs without a count. |
| `rustward_log_truncations_total` | Counter of completed runs with a positive truncation flag on either stream. |
| `rustward_storage_failures_total` | Counter of observed state and task-log storage failures by component. |
| `rustward_expired_runs_total` | Counter of completed runs removed by age-based retention. |
| `rustward_service_logs_dropped_total` | Counter of records dropped due to a full queue or failed service log write. |

Counters and histogram observations reset at process start. Reopening retained
history does not replay earlier transitions into the counters. A completion
committed during recovery counts in the recovering process. Offline state
commands emit logs for their own operations.

Run `just telemetry-test` for synthetic state, HTTP, and stalled sink checks.
The Docker acceptance web case verifies separate proxy credentials, parses the
metrics, and traces a synthetic run through service logs.

### Health, readiness, and pilot rehearsal

Both probe routes return JSON with `ok` and `state`. A passing probe returns
HTTP 200. A failing probe returns HTTP 503. Health requires a service-loop
heartbeat within 10 seconds and a healthy controller. A paused, draining, or
review-blocked service can still be healthy. Readiness requires health and an
idle dispatch state or ordinary execution capacity in use. A pause, drain,
unresolved review, or log capacity block makes readiness fail.

The pilot alerts when a run fails, times out, or ends in interruption, when a
schedule gap is recorded, and when Rustward becomes unhealthy or unreachable.
A pause, drain, full execution capacity, or readiness block alone is displayed
in operator status. A run interruption can still alert while dispatch waits
for review. Alert rules use process-local measurements. An event followed by
a restart between scrapes can be missed.

`just pilot-test` sends synthetic alerts through Prometheus and Alertmanager to
a loopback SMTP sink. The full `just acceptance-container` suite also proves
proxy access and rehearses a systemd restore. It checks that the held state
rejects startup, a second scheduler cannot open the same state database, and
the restored service admits only a new occurrence after explicit resume.
On separate state databases, keep schedule ownership exclusive by stopping the
original before starting the replacement.

Live metrics collection and delivery through the deployment relay require a separate
deployment check in the deployment repository.
