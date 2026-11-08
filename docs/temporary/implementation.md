# Implementation work

Status: Temporary work list. Configuration validation, request input validation,
argument generation, durable manual admission, and queued cancellation are implemented. The execution
core is not implemented. A separate supervision interface and systemd user-manager
adapter and bounded per-run output capture are implemented. Real host verification,
aggregate log reservations, and execution integration remain open.
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

1. Specify dispatch, launch recovery, and the remaining persisted lifecycle
   transitions under the accepted execution contracts. Manual key verification,
   atomic admission, and queued cancellation with separate phase and result
   storage are implemented. Configuration and argument
   mapping use the implemented TOML version 1 contract; see its
   [format change](configuration.md#format-change-and-compatibility). Further
   schema changes need an explicit compatibility and migration decision.
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

## Remaining specifications and evidence

The policy interview is complete for these contract groups. The following work
must translate accepted behavior into testable mechanisms, not reopen it:

- Integrate the implemented manual admission library with service safety controls
  and verified prepared runtime selection. Preserve the [admission contract](../ARCHITECTURE.md#manual-admission-library).
  Define restore tooling and monitoring of permanent-record growth. Transport
  parsing and global metadata/buffer bounds remain open.
- Define the remaining state-transition table and durable launch protocol.
  Preserve the implemented [queued cancellation transition](../ARCHITECTURE.md#queued-cancellation-library).
  Reconcile recorded runs against systemd evidence after a crash. Test loss of
  confirmation before and after launch; do not automatically retry uncertain work.
- Specify schedule fields and supported cron syntax, including wildcard handling.
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
- Specify atomic log-capacity reservation, release, record/log deletion, and
  retention age calculation. Test protected evidence and actual storage faults.
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

- **P01:** Use an internal Rust scheduler with an established cron parser;
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
- **P04:** Reload configuration atomically through an explicit local action.
  Store schedule identity/revision with checkpoints and persistent pause state.
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
