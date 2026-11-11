# Implementation work

Status: Temporary work list. Configuration, admission, dispatch, stopping,
review, runtime verification, and the [execution controller](controller.md) are
implemented as libraries. The controller connects the systemd adapter, pinned
host dependencies, capture, and log accounting. Isolated-host acceptance,
retention, and the service loop remain open.
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

1. Run the complete A01–A10 suite with `just acceptance-ssh`. The
   [remaining cases](remaining-acceptance.md) are implemented. Docker is absent
   from the devbox. The user reported A01–A05 passing; the full successful bundle
   has not been copied here.
2. Review the complete copied evidence for A06–A10 before extending the execution
   core. Synthetic library tests and direct playbook smoke checks are not host
   acceptance evidence.
3. Specify remaining task resource controls, custom environment/secret references,
   and metadata/buffer bounds. Connect the controller to the future service and
   implement retention reclamation. Keep scheduler and UI integration separate.
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
Schema version 7 preserves version 1–6 identities and history. Tests cover lock
ownership, competing dispatchers, cancellation races, FIFO/capacity, write rollback,
process exit at launch boundaries, missing/changed identity, first-result retention,
cleanup/output gates, and durable review. No test starts a real task.

[Runtime verification](runtime.md), protected paths, and separate writable
directories are implemented. The [controller](controller.md) now connects actual
service/manager UID binding, pinned host manifests, controlled environment,
capture, stopping, and atomic output accounting. The [local harness](acceptance-runner.md)
covers ordinary-command, local-playbook, and local recovery checks. The user
reported A01/A02 passing. A03 is implemented but unrun; see the
[Ansible acceptance specification](ansible-acceptance.md). Retention reclamation remains separate work. Review
can be cleared only after local completion and acknowledgement of unchanged
evidence. A dropped permit is not reissued.

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
  Validate process-boundary enforcement and cleanup evidence. Integrate the
  implemented acknowledgement record.
- Verify controller user binding and host dependencies on the isolated host.
  Specify safe removal of run work directories.
- Verify controller log accounting and implement retention cleanup,
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
