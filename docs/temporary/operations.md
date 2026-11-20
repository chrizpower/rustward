# Operational contracts

Scheduling and explicit reload are implemented. Age-based retention, restore,
and telemetry are accepted future work. See the [service interface](service.md).

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

## Age-based retention

Use the configured retention age for eligible completed records and logs.
Preserve queued and active records, unresolved review records and their
retained output, and permanent admission records. Capacity-based log
reclamation is implemented separately. See
[log reclamation](execution.md#log-reclamation) and [Gravity P07](../GRAVITY.md#p07-enforce-configurable-resource-bounds).
Define age calculation and deletion behavior, including unknown historical
timestamps. Verify against storage faults. Global metadata bounds and
monitoring of permanent-record growth remain open.

## Application-state restore contract

These decisions concern backups of Rustward's own application state, not backup
tasks executed by Rustward. The user accepted the behavior on 2026-11-08. Backup
tooling and production deployment remain separate work.

Restore starts with admission and dispatch closed. Review restored queued and
active runs before any can execute: they may have completed after the backup. Do
not automatically replay them. Create a new manual-key issuer generation. Known
old keys identify their restored admission records. Unknown keys from older
generations are rejected. Fresh requests use the new generation.

Restore must be an explicit procedure. Arbitrary rollback of a database cannot
be assumed safe. Define backup consistency, generation verification, and the
per-run review record before implementing restore. Normal restarts do not by
themselves require a new generation. No separate admission ledger is required by
this decision.

Test isolated restore with synthetic history. Verify that unknown old-generation
keys cannot admit work and that restored queued runs stay blocked until review.

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
