# Implementation work

Status: Temporary work list. Configuration validation, request input validation,
and argument generation are implemented; the execution backend is not.
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

1. Specify atomic admission, key verification, persisted transitions, and launch
   recovery under the accepted execution contracts. Configuration and argument
   mapping are already implemented; extend their schema only through an explicit
   compatible contract change.
2. Select the Linux/systemd baseline, manager, and narrowly scoped permissions.
   Specify runtime layout, output capture, environment and secret references,
   task resource controls, and remaining metadata/buffer bounds before integration.
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

- Define key format, issuer-generation verification, canonical request encoding,
  atomic admission, and bounded rejection diagnostics. Test concurrent retries
  and interrupted admission. Issued but unadmitted keys need no permanent
  admission record; prove their authenticity through the selected mechanism.
- Define the complete state-transition table and durable launch protocol.
  Reconcile recorded runs against systemd evidence after a crash. Test loss of
  confirmation before and after launch; do not automatically retry uncertain work.
- Specify schedule fields and supported cron syntax, including wildcard handling.
  Persist evaluation checkpoints together with occurrence decisions. Demonstrate
  reliable coverage; report intervals with insufficient evidence as gaps. Test
  clock changes, outages, schedule edits, and first installation.
- Select systemd baseline, manager, and permissions on an isolated test host.
  Validate process-boundary enforcement and cleanup evidence. Define the review
  record with affected runs, evidence, uncertainty, decision, and reason.
- Specify runtime manifest, paths, ownership, canonical revision encoding,
  and safe removal of run work directories after completion.
- Specify atomic log-capacity reservation, release, record/log deletion, and
  retention age calculation. Test protected evidence and actual storage faults.
- Specify a consistent application-state backup and explicit restore procedure.
  Test that missing old-generation keys cannot admit work and restored queued
  records cannot execute without review.

These are engineering specifications and validation gates. Host-specific
choices, new behavior trade-offs, and any conflicts with accepted policy need
further review. Backup tooling, production integration, and workload-specific
evidence remain outside this interview. No execution behavior is verified yet.

## Retained mechanism proposals

These unresolved proposals came from the original requirements reference. They are not
accepted rules. Apply accepted execution contracts where a proposal overlaps;
review a proposal before implementing it.

- **P01:** Use an internal Rust scheduler with an established cron parser;
  systemd timers remain an alternative only if they preserve accepted occurrence,
  admission, and missed-work behavior. Keep one schedule owner. Consider SSE or
  bounded polling for output, and derive per-run unit identity from the run ID.
  Transient services and `KillMode=control-group` are proposed systemd mechanisms.
- **P02:** Use a SQLite transaction for request identity and run admission, and
  persist queue order for dispatch. Show queued runs before resuming. Preserve
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
