# Durable dispatch and launch recovery

Status: Implemented library contract; tests use fake supervision only. The user
accepted FIFO selection, cancellation race handling, and recovery rules on
2026-11-08. This document does not authorize real execution. Transfer stable
interfaces to ARCHITECTURE.md and meanings to GLOSSARY.md before removal; retain
open integration work. Follow execution.md and Gravity P02, P03, P06, and P07.

## Boundary and prerequisites

The library implements database transitions and a one-call launch protocol.
The [controller](controller.md) connects runtime/host verification, bounded
capture, stopping, and the systemd adapter. Tests use synthetic supervision.
There is no daemon, service dispatch loop, submission command, scheduler, or HTTP
endpoint. Isolated-host acceptance remains required before real use. Callers
must preserve admitted values and use the same shared Safety across handles.

A dispatch session holds an exclusive nonblocking OS lock on the database inode.
Other admission connections can continue; a second dispatch session cannot open.
Keep the protected database in place: replacing or restoring it while sessions
exist is unsupported. A launch permit retains the lock until consumed or dropped.
This prevents a new owner recovering while an old permit could still start work.

## Transactions and launch permission

New databases start with dispatch paused. Admission closure and dispatch pause
remain independent. An immediate SQLite transaction selects the oldest queued
run, checks pause, review blocks, execution capacity, and log accounting capacity,
and changes Queued to Launching. The same transaction records launch intent and
the full per-run log allowance. Commit before exposing a launch permit.

The permit contains the retained executable, working directory, generated
arguments, runtime identity, and original run ID. It is not cloneable or
reconstructed on reopen. Consume it for at most one supervisor start call. No
transaction stays open during the call. The permit is valid only in its owning
session. A failed intent commit produces no permit and no supervisor call. If a review
block appears after a permit was granted, `start` returns `Closed` without calling
the supervisor. The consumed permit is not reissued; recovery preserves its intent.
Dispatch pause prevents further claims; an already dispatched permit belongs to
active work. Review and storage failures still prevent its start.

Log accounting is a conservative prerequisite for this library: reserve each
run's full allowance against one aggregate limit. Keep reservations through
uncertainty and cleanup. On completion replace the allowance with the declared
synchronized retained byte count. The controller uses the internal claim path
that reclaims completed-run logs. The public `Session::claim_next` uses accounting
only. Limits come from the caller.

Cancellation and dispatch share the SQLite write lock. Cancellation first means
Completed/Cancellation and no launch permit. Dispatch first means `NotQueued`
from `cancel_queued`; active stopping uses `Session::cancel_active` and `poll`. FIFO never bypasses a block.
Reducing capacity does not evict active runs or alter admitted snapshots.

## Log reclamation

The controller reclaims logs when the next queued run needs capacity. It removes
eligible completed-run logs in completion order, with admission sequence as the
tie breaker. It preserves output needed for unresolved review and charges for
active runs. If eligible logs cannot free enough space for the full reservation,
the controller leaves the run queued.

The controller removes the capture files and synchronizes their directories
before it releases the charge. The charge update and new reservation commit
together. After an interruption, already absent files can be handled again
without releasing capacity twice. A removal failure invokes P06. Run history,
admission records, and historical capture evidence stay available. The current
log charge can be zero even when historical evidence records captured bytes.
Age-based cleanup remains open.

## State and evidence

| Evidence | Recorded effect |
| --- | --- |
| Committed claim | Launching; start intent and slot/log reservations retained |
| Supervisor accepts start request | Record Accepted; remain Launching until observed Running |
| Definite supervisor rejection | Cleaning up/Failure; record that no process started |
| Conflict, lost reply, or supervisor error | Cleaning up/Interruption unless a result was already decided; require review |
| Matching Running observation | Running, only if no result was already decided |
| Matching exit observation | Cleaning up; first result is Success for code 0, otherwise Failure |
| Missing or uncertain observation, or changed execution identity | Preserve original identity and prior result; otherwise Interruption; require review |
| Confirmed local cleanup | Retain cleanup evidence; never infer it from a missing unit |
| Cleanup and synchronized output confirmed | Completed with the retained result; release execution slot, retain log accounting and review block |

Store the first observed exit code or signal as evidence. A conflicting later
exit requires review without replacing that evidence or the first result.
Store execution invocation ID and cgroup path before further observation,
stop, or release can rely on them. A start acceptance is not process-start
proof. A definite rejection proves no process was started by that call; conflict
and transport failure do not. Do not claim completion before output has been
synchronized. This slice accepts the caller's output confirmation; capture
integration remains a gate. Stopping is separate explicit polling; release remains the caller’s responsibility.

## Recovery and failure

On a new dispatch session, recover all unfinished dispatched runs in one
transaction. Preserve a durably decided result; otherwise record Interruption.
Move them to Cleaning up and require review. Preserve accepted queued work,
original identities, arguments, runtimes, and resource reservations. Never
reconstruct a permit or call start for recovered work, including a crash after
intent commit but before the original start call.

A review block survives later observations, cleanup, completion, and reopen.
[Acknowledgement](review.md) records one note for one completed run and its
reviewed evidence, then clears that run’s block. Pausing
or resuming dispatch cannot clear review. Absence of a systemd unit alone does
not release a slot. Recovery polling stops uncertain local work. The service must obtain cleanup
evidence under the accepted contracts.

A storage error closes the affected library handle. A busy error while saving
a start reply or observation also closes dispatch: the external evidence has
not been retained. No later claim or start is allowed on that handle. Do not return durable
success after a failed reply/evidence commit. The intent remains available for
recovery. `safety::Safety` supplies a shared process-local failure gate. Open all
service admission and dispatch connections with `open_with_safety` and clones of
the same gate. Storage errors and failed dispatch evidence writes set it without
another database write. New admissions, claims, and already granted starts are
blocked. Matching admitted retries retain their original identities on healthy
connections. The gate cannot be cleared; reopening with the same gate keeps the
block. Default `open` calls each create a separate gate and do not connect a
service's failure controls. Polling can stop active work without another write. Recovery and review after
unrecorded evidence remain conservative; see [stopping](stopping.md).

## Compatibility and required evidence

Use database schema version 7. Preserve all version 1–6 run fields, results,
sequence high-water marks, issuer data, permanent admissions, snapshots, and
revision bytes. Older binaries reject version 7. Task configuration remains
version 2. New snapshots include timing defaults; stored snapshots, revisions,
and request fingerprint encoding remain unchanged.

Test exclusive ownership, competing claims, FIFO, capacity, pause, admission
independence, cancellation races, commit rollback, and retained launch inputs.
Inject crashes before intent, after intent, and after supervisor acceptance;
verify no second start. Test lost replies, definite rejection, identity mismatch,
missing units, result preservation, cleanup/output gating, restart review, and
migration failure. These tests do not prove host power-loss durability, real
systemd cleanup, runtime validity, or service-wide P06 handling.

## Rust interface

`Session::open` takes a database path and `dispatch::Limits`. Counts are `u32`;
log limits must fit a nonnegative SQLite `i64`. `set_paused` changes only dispatch
pause. `claim_next` returns `Claim::Ready(Permit)` or a fixed `Block` reason.
`start` consumes a permit, a mutable supervisor, and borrowed output descriptors.
The caller owns descriptor closure and capture synchronization. A supervisor error
is recorded as an uncertain outcome. `observe` records supplied evidence; it
does not poll the manager. `record` exposes retained launch/exit/cleanup/review
evidence, and `run_state` exposes phase/result. `finish` accepts the synchronized
retained-byte count and never clears review. `Store::acknowledge` requires local
completion and unchanged evidence; it does not alter run results or other gates. Repeated finish is an invalid
transition. Cross-session permits are rejected. No transport API is provided.

`fail_before_start` consumes an unstarted permit and records Failure with a fixed
launch-failure code and confirmed no-start cleanup. Output handling must still
finish before slot release. [Runtime verification](runtime.md) supplies
prepare_runtime and start_prepared. The [controller](controller.md) connects
worker preparation, host checks, and capture. Isolated-host acceptance remains gated.

[Active stopping](stopping.md) adds output failure, stop reasons, timestamps,
and timeout polling. The controller supplies coordination; no daemon is provided.

`fail_capture_setup` records one of exists/storage/resources before a process
starts. Storage sets Safety. `finish_evidence` accepts final per-stream retained
and observed counts, truncation, synchronization, and recovery status. It rejects
invalid counters and successful completion claims from failed or recovered
output. Evidence, log accounting, and completion commit together. Nullable new
fields are omitted when absent in review encoding 1. Historical review records,
snapshots, revision bytes, fingerprints, and matching retries remain unchanged.
