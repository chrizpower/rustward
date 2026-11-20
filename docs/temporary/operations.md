# Operational contracts

Scheduling, live reload, age-based retention, restore, and telemetry are
accepted future work. The local service implements manual work and planned
drain. Its interface is in [service.md](service.md).

## Scheduling

Schedule definitions come only from TOML files under the accepted
[file contract](configuration.md#accepted-file-layout). The database retains
runtime state, not editable schedule definitions. A schedule ID is its path
relative to the schedule root without `.toml`. A rename or move within the root
removes one identity and adds another. Do not transfer old pause state or
occurrence history automatically. Preserve accepted runs and old records. Apply
first-installation behavior to the new schedule without replaying missed work.
Initial pause and checkpoint mechanisms remain open.

At an unchanged path, the schedule ID stays stable. Occurrence identity is
schedule ID plus UTC due time. Record the schedule-definition revision separately. Editing a schedule
does not make an already handled due time eligible again. Updated definitions
apply to future occurrences.

Use the strict five-field UTC cron subset in the
[configuration contract](configuration.md#schedule-schema). Allow numeric values,
lists, ranges, and standalone `*`. Reject steps and extensions. Day of month or
day of week must be exactly `*`. The user accepted this restriction in the cron
review on 2026-11-08, replacing the earlier acceptance of combined day fields
with OR matching. Use an established parser/evaluator for accepted expressions.

Subset validation and library matching tests are implemented with `croner`.
Scheduler execution, durable occurrence decisions, and clock-change handling
remain future work.

Allow at most one queued run per schedule. Further due occurrences are recorded
as skipped while that queued run remains. An active run may have one queued
successor. The shared queue limit still applies.

Persist evaluation checkpoints together with occurrence decisions. Report
intervals without reliable evaluation as schedule gaps, with a missed count
only when known. Skip missed work on restart. Apply the accepted [control
effects](../GRAVITY.md#accepted-operational-controls). Test outages, clock changes
in both directions, schedule edits, and first installation.

## Live reload

Validate a consistent complete candidate set before activation. Keep the
previous set if validation fails. Preserve accepted runs and handled
occurrences. The activation trigger and consistent-read mechanism remain open.
See the [configuration contract](configuration.md#validation-and-retained-work).

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
