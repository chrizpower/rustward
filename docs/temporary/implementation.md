# Implementation work

Status: Temporary work list. Configuration validation, request input validation,
argument generation, manual admission, queued cancellation, and the durable
dispatch protocol are implemented as libraries. Real execution is not integrated. A separate supervision interface and systemd user-manager
adapter and bounded per-run output capture are implemented. Real host verification,
real log-capacity integration, retention, and execution integration remain open.
Accepted behavior is currently recorded in [execution.md](execution.md) until
absorbed into the uppercase documents; this file tracks the work.
Follow [Gravity](../GRAVITY.md) and [requirements](requirements.md).

## Completion and removal

As each component is implemented, move stable interface and operational details
into ARCHITECTURE.md, GRAVITY.md, or GLOSSARY.md, update implementation status, and record
verification in tests or the appropriate acceptance record. Remove completed
work items. Delete this file after all work is completed or explicitly deferred
to a maintained tracker, and update every inbound link. Do not delete unresolved
requirements or acceptance evidence. See [folder guidance](README.md).

## Next implementation work

1. Complete the integration gates in the [dispatch contract](dispatch.md).
   Durable intent, exclusive ownership, FIFO claims, capacity accounting, retained
   launch data, supervisor evidence, and conservative recovery are tested with a
   fake supervisor. [Active stopping](stopping.md), timeouts, shared failure gates,
   and timestamps are implemented. Specify acknowledgement records before adding
   review release. Connect capture and stopping to the future service.
2. Verify the selected Debian 13/systemd 257 dedicated user manager and its
   permissions on an isolated host.
   Specify runtime layout, environment and secret references,
   task resource controls, and remaining metadata/buffer bounds before integration.
   Integrate the [implemented per-run capture library](capture.md), including its
   early failure signal, with durable dispatch and service-wide P06 controls.
3. Implement the execution core with synthetic inputs. Select the required Rust
   libraries and prove the failure cases below on an isolated systemd host.
4. Review a sanitized Semaphore export before claiming workload compatibility.
   Select the license before release. Access, migration, and deployment remain
   separate gates, not prerequisites for synthetic configuration tests.

Execution-core evidence must cover failed exec, crash windows, duplicate submissions,
queue bounds, output flooding, disk failure, detached children, concurrent run
isolation, and uncertain external effects. Test systemd on an isolated Linux
system with the selected manager. Do not expose the production host bus or grant
privileged access to the devbox merely to make integration tests pass.

## Configuration layout work

The [file contract](configuration.md#accepted-file-layout) is implemented by
`config::load` and the validation CLI. Version 2 separates service settings,
task files, and schedule files. Nested relative paths supply IDs. Complete-set
validation includes the strict cron subset, task references, and fixed input values.
`croner` parses allowed expressions. UTC matching tests cover weekday selection,
month/year boundaries, and leap days; no scheduler is connected.
Input validation and argument generation retain their existing behavior.

Tests cover nested equal basenames, invalid paths, symlinks including ancestors,
missing and empty roots, file/entry/depth limits, invalid references and values,
observed concurrent edits, root relocation, definition moves, and retained
admission identity. Task snapshots keep their existing encoding; new complete-set
revisions include schedules with a separate version prefix. Stored revisions,
request fingerprints, and admitted runs are not rewritten.

Remaining work: activation and its consistent-read mechanism, reload trigger,
scheduler execution, occurrence history, pause-state handling, first-installation
checkpoints, and service-setting integration. A renamed schedule must not inherit
pause state or replay missed occurrences. Editing a schedule must not make a
handled occurrence eligible again. Loader tests do not prove these runtime rules.
Production migration and deployment remain outside repository implementation.

## Dispatch evidence and remaining work

The [dispatch contract](dispatch.md) owns the implemented API and its prerequisites.
Schema version 4 preserves version 1–3 identities and history. Tests cover lock
ownership, competing dispatchers, cancellation races, FIFO/capacity, write rollback,
process exit at launch boundaries, missing/changed identity, first-result retention,
cleanup/output gates, and durable review. No test starts a real task.

Next: verify prepared runtimes, connect capture and the stopping library,
implement acknowledgement and retention reclamation, and pass isolated systemd
host checks. The service must enforce all gates before real execution. A review
block currently has no acknowledgement API. A dropped permit is not reissued.

## Telemetry work

Implement the accepted [telemetry contract](telemetry.md): structured service
logs and a private Prometheus-compatible `/metrics` endpoint for the monitoring service. Neither is
implemented. Task output capture remains separate.

Specify event fields, log transport and retention, metric names/types/units and
bounded labels, duration and reset semantics, telemetry settings, and endpoint
access. Resolve service-log sink failure handling against P06. Set numeric
resource bounds and overload behavior. Instrument durable decisions without
counting matching retries as new runs. Add the contract's synthetic failure,
sensitive-data, exposition, and load checks. Collector configuration and proof
of collection from the monitoring service belong in separately authorized deployment work.

## Remaining specifications and evidence

The policy interview is complete for these contract groups. The following work
must translate accepted behavior into testable mechanisms, not reopen it:

- Integrate the implemented manual admission library with service safety controls
  and verified prepared runtime selection. Preserve the [admission contract](../ARCHITECTURE.md#manual-admission-library).
  Define restore tooling and monitoring of permanent-record growth. Transport
  parsing and global metadata/buffer bounds remain open.
- Integrate the implemented durable launch and stopping protocols with review.
  Preserve the implemented [queued cancellation transition](../ARCHITECTURE.md#queued-cancellation-library).
  Integrate recorded recovery decisions with real systemd cleanup. Keep the
  tested no-retry rule at every launch boundary.
- Integrate `croner` with the scheduler through the accepted strict subset check.
  Persist evaluation checkpoints together with occurrence decisions. Demonstrate
  reliable coverage; report intervals with insufficient evidence as gaps. Test
  clock changes, outages, schedule edits, and first installation.
- Verify the selected systemd baseline, manager, and permissions on an isolated
  Debian 13 test host. The [supervision adapter contract](../ARCHITECTURE.md#supervision-interface-and-systemd-adapter)
  is implemented with synthetic tests; its opt-in host test remains unrun.
  Validate process-boundary enforcement and cleanup evidence. Define the review
  record with affected runs, evidence, uncertainty, decision, and reason.
- Specify runtime manifest, paths, ownership,
  and safe removal of run work directories after completion.
- Integrate conservative dispatch log accounting with capture, retention cleanup,
  record/log deletion, and retention age calculation. Test protected evidence and actual storage faults.
- Specify a consistent application-state backup and explicit restore procedure.
  Test that missing old-generation keys cannot admit work and restored queued
  records cannot execute without review.

These are engineering specifications and validation gates. Host-specific
choices, new behavior trade-offs, and any conflicts with accepted policy need
further review. Backup tooling, production integration, and workload-specific
evidence remain outside this interview. Admission storage, queued cancellation, and schema migration are tested; process execution behavior is not verified.

## Retained mechanism proposals

These unresolved proposals came from the original requirements reference. They are not
accepted rules. Apply accepted execution contracts where a proposal overlaps;
review a proposal before implementing it.

- **P01:** The cron review selects an established parser/evaluator; `croner` now
  parses the supported subset. An internal scheduler remains proposed;
  systemd timers remain an alternative only if they preserve accepted occurrence,
  admission, and missed-work behavior. Keep one schedule owner. Consider SSE or
  bounded polling for output, and derive per-run unit identity from the run ID.
  The separate supervision adapter now implements transient services and
  `KillMode=control-group`. Host evidence remains a gate for execution integration.
- **P02:** SQLite transactions now record manual request identity and queued runs,
  including insertion order. Dispatch integration remains future work. Show
  queued runs before resuming. Preserve
  accepted runs across schedule removal/reload. Any proposed task-specific waiver
  of uncertainty blocking needs explicit review against accepted P02; it is not
  an existing exception.
- **P03:** Display the selected runtime revision and test dependency updates and
  failed preparation. Keep runtime references protected from premature removal.
- **P04:** Use an explicit local action to trigger reload; the trigger remains
  proposed. Whole-set validation and activation are now accepted in the file
  contract. Store schedule identity/revision with checkpoints and persistent
  pause state.
  Report the lack of a prior checkpoint on first installation. Test forward and
  backward clock changes without replaying admitted occurrences.
- **P05:** Use a supported SQLite backup procedure with consistent logs and
  runtime references. Report missing logs. Record per-run restore release or
  cancellation decisions, and verify that no old executor or schedule owner is
  active before enabling execution. Test schema rollback compatibility.
- **Web and operations:** Test invalid inputs, output escaping, access checks,
  CSRF, reconnects, and notification failures. Measure resource and latency
  targets before acceptance; the proposed targets are not guarantees.

Original C01 and accepted P01/P02/P03 rules are maintained in Gravity. Superseded
key-expiry and stop-on-planned-shutdown proposals are not implementation options.
