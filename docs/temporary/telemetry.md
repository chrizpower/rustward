# Telemetry contract

Status: Accepted future contract. Source: The user requested service logs and a
Prometheus-compatible endpoint for collection by the monitoring service, then authorized this
contract on 2026-11-08. Neither structured service logging nor a metrics endpoint
is implemented. Existing task output capture is separate.

Scope: Rustward service telemetry and its collection interface. Production
collector configuration belongs in `the deployment repository`. This contract does not
authorize deployment or changes on the monitoring service. Before removing this temporary file,
transfer lasting interface and verification details to ARCHITECTURE.md and policy
to GRAVITY.md; preserve open work and update links. See [folder guidance](README.md).

## Service logs

Provide structured service logs for startup and shutdown, configuration
acceptance and rejection, admission, dispatch, run completion, recovery,
operational controls, and state or task-log storage failures. Report only
implemented and observed events; a launch request is not proof of process start.
A successful admission log must follow the durable admission decision.

Records need a timestamp, severity, stable event name, component, and bounded
reason or result fields. Where relevant, include task, schedule, and run IDs for
correlation with retained state. These IDs are log fields, not metric labels.
Use established run phases and results from the [glossary](../GLOSSARY.md).

Keep task stdout/stderr in the existing [capture interface](capture.md). Do not
copy task output, submitted input values, command arguments, credentials,
request keys, or secret-bearing error details into service logs. Apply the same
restrictions at debug levels. Use bounded reason codes instead of raw error
strings where errors can contain payloads. Logs support diagnosis; they do not
replace durable admission records, run history, or acknowledgement evidence.

## Metrics collection

Provide a private, read-only HTTP `GET /metrics` endpoint with a
Prometheus-compatible exposition format and matching content type. the monitoring service must be
able to scrape it without a browser session. A scrape must not admit work,
change controls, or execute a task. Keep metrics available during admission or
dispatch blocks where the service can still serve requests, so the block remains
observable.

Keep access private and restrict it to authorized monitoring access. Specify the
listener, access mechanism, and collector route before deployment. Do not infer
the monitoring service's address, credentials, collector software, or network access from its name.
Service telemetry settings belong in `rustward.toml`; no telemetry fields are
implemented in the current schema. Exact keys and defaults remain open.

Expose these measurements as their owning components are implemented:

| Measurement | Required meaning |
| --- | --- |
| Queue depth | Current accepted runs waiting for dispatch |
| Active runs | Current runs holding an execution slot, including launch and cleanup |
| Admissions and rejections | New durable admissions and rejected submissions, with bounded trigger/reason categories; a matching retry is not a new admission |
| Completed runs | Recorded completions by result; cancellation before dispatch is included |
| Run duration | A defined duration distribution; specify start/end points and treatment of queued cancellation before implementation |
| Skipped occurrences | Known skipped schedule occurrences by bounded reason; unknown schedule gaps must not be converted to invented counts |
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

## Bounds and failure behavior

Bound log record size, log buffering, metric series, scrape work, and concurrent
scrapes. Specify numeric bounds and overload handling before implementation.
Telemetry must not introduce an unbounded queue or hold an admission transaction
open while waiting for a remote collector. A failed scrape or an unreachable the monitoring service
does not change a run result or prove that execution failed.

Existing P06 behavior for state and retained task-log storage failures remains
mandatory. The response to service-log sink failure, its relation to P06, and
how to expose telemetry loss remain open decisions. Do not silently weaken P06
or equate a remote collection outage with a local state-storage failure.
Service-log transport, encoding, rotation, and retention also need specification;
per-run task-log limits do not define service-log retention.

## Verification and implementation gates

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
