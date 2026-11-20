# Durable execution

The dispatch library owns state transitions, launch permission, stopping, and
capacity accounting. The [controller](service.md#execution-controller) connects
it to the local service, prepared runtimes, systemd, and capture.
See [Gravity](../GRAVITY.md) for P02, P03, P06, and P07.

The user accepted FIFO selection, cancellation race handling, conservative
recovery, and the default timeout on 2026-11-08.

## Run lifecycle

| Phase | Meaning |
| --- | --- |
| Queued | Waiting for dispatch |
| Launching | Holds a slot while it waits for program start confirmation |
| Running | Program start is confirmed. Exit evidence is pending |
| Cleaning up | Launch or execution has ended, or stopping has begun. Local cleanup is pending |
| Completed | Cleanup confirmed and final result recorded |

Results distinguish success, failure, cancellation, timeout, and interruption.
Use exit code 0 for success. Nonzero exit codes and unexpected signals mean
failure unless a prior stop decision applies. Use a task wrapper for a different
exit convention. The [systemd adapter](execution-internals.md#supervision-interface-and-systemd-adapter)
owns the local process boundary. Tasks must leave no local daemons.

## Boundary and prerequisites

A dispatch session holds an exclusive nonblocking OS lock on the database inode.
Other admission connections can continue. A second dispatch session cannot open.
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
the supervisor. The consumed permit is not reissued. Recovery preserves its intent.
Dispatch pause prevents further claims. An already dispatched permit belongs to
active work. Review and storage failures still prevent its start.

Log accounting is a conservative prerequisite for this library: reserve each
run's full allowance against one aggregate limit. Keep reservations through
uncertainty and cleanup. On completion replace the allowance with the declared
synchronized retained byte count. The controller uses the internal claim path
that reclaims completed-run logs. The public `Session::claim_next` uses accounting
only. Limits come from the caller.

Cancellation and dispatch share the SQLite write lock. Cancellation first means
Completed/Cancellation and no launch permit. Dispatch first means `NotQueued`
from `cancel_queued`. Active stopping uses `Session::cancel_active` and `poll`. FIFO never bypasses a block.
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
Age-based cleanup remains open. See [retention](operations.md#age-based-retention).

## State and evidence

| Evidence | Recorded effect |
| --- | --- |
| Committed claim | Record Launching, start intent, and slot and log reservations |
| Supervisor accepts start request | Record Accepted. Remain Launching until observed Running |
| Definite supervisor rejection | Record Cleaning up and Failure with no process start |
| Conflict, lost reply, or supervisor error | Cleaning up/Interruption unless a result was already decided. Require review |
| Matching Running observation | Running, only if no result was already decided |
| Matching exit observation | Record Cleaning up. The first result is Success for code 0 and Failure for other exits |
| Missing or uncertain observation, or changed execution identity | Preserve original identity and prior result. Otherwise Interruption. Require review |
| Confirmed local cleanup | Retain cleanup evidence. Never infer it from a missing unit |
| Cleanup and synchronized output confirmed | Completed with the retained result. Release execution slot, retain log accounting and review block |

Store the first observed exit code or signal as evidence. A conflicting later
exit requires review without replacing that evidence or the first result.
Store execution invocation ID and cgroup path before further observation,
stop, or release can rely on them. A start acceptance is not process-start
proof. A definite rejection proves no process was started by that call. Conflict
and transport failure do not. Do not claim completion before output has been
synchronized. A low-level caller supplies output confirmation, polls the manager,
and releases the unit reference. The controller coordinates these steps.

## Recovery and failure

On a new dispatch session, recover all unfinished dispatched runs in one
transaction. Preserve a durably decided result. Otherwise record Interruption.
Move them to Cleaning up and require review. Preserve accepted queued work,
original identities, arguments, runtimes, and resource reservations. Never
reconstruct a permit or call start for recovered work, including a crash after
intent commit but before the original start call.

A review block survives later observations, cleanup, completion, and reopen.
[Acknowledgement](execution-internals.md#administrator-review-and-acknowledgement) records one note for one completed run and its
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
connections. The gate cannot be cleared. Reopening with the same gate keeps the
block. Default `open` calls each create a separate gate and do not connect a
service's failure controls. Polling can stop active work without another write.
See [active stopping](#active-stopping) for failure handling.

## Rust interface

`Session::open` takes a database path and `dispatch::Limits`. Counts are `u32`.
Log limits must fit a nonnegative SQLite `i64`. `set_paused` changes only dispatch
pause. `claim_next` returns `Claim::Ready(Permit)` or a fixed `Block` reason.
`start` consumes a permit, a mutable supervisor, and borrowed output descriptors.
The caller owns descriptor closure and capture synchronization. A supervisor error
is recorded as an uncertain outcome. `observe` records supplied evidence. It
does not poll the manager. `record` exposes retained launch/exit/cleanup/review
evidence, and `run_state` exposes phase/result. `finish` accepts the synchronized
retained-byte count and never clears review. `Store::acknowledge` requires local
completion and unchanged evidence. It does not alter run results or other gates. Repeated finish is an invalid
transition. Cross-session permits are rejected. No transport API is provided.

`fail_before_start` consumes an unstarted permit and records Failure with a fixed
launch-failure code and confirmed no-start cleanup. Output handling must still
finish before slot release. [Runtime verification](runtime.md) supplies
`prepare_runtime` and `start_prepared`. The [controller](service.md#execution-controller) connects
worker preparation, host checks, and capture. Isolated-host acceptance remains gated.

`fail_capture_setup` records one of exists/storage/resources before a process
starts. Storage sets Safety. `finish_evidence` accepts final per-stream retained
and observed counts, truncation, synchronization, and recovery status. It rejects
invalid counters and successful completion claims from failed or recovered
output. Evidence, log accounting, and completion commit together. Nullable new
fields are omitted when absent in review encoding 1. Historical review records,
snapshots, revision bytes, fingerprints, and matching retries remain unchanged.

## Active stopping

### Task settings and retained data

Version 2 task files accept `timeout_seconds` (default 3600) and
`cleanup_grace_seconds` (default 30). Both are whole seconds from 1 through
4294967295. Zero and disabled timeouts are not supported for new definitions.
New admissions retain both values in normalized task snapshots. Explicit defaults
and omitted defaults have equal revisions. Timing edits affect new admissions.
Stored task IDs, snapshots, revisions, fingerprints, and matching retries remain
unchanged. Legacy snapshots without these fields retain no timeout and a
30-second cleanup grace. No history is rewritten.

The timeout starts at dispatch and includes launch. A monotonic `Instant` measures
execution and cleanup time. UTC wall-clock changes do not change deadlines.
`Session::poll(supervisor, now)` requires explicit calls with monotonic time.
The low-level session has no background timer. The caller must poll promptly,
including after failures. Polling delay can delay stopping. Recovery never resets a timeout
or starts recovered work: it retains a decided result or records Interruption,
requires review, and begins cleanup.

### Decisions and stopping

`cancel_active` records Cancellation for an unfinished dispatched run unless a
result already exists. It returns Recorded, AlreadyEnded, or NotActive. Queued
cancellation remains `Store::cancel_queued`. Timeout, exit, and cancellation use
SQLite transactions. The first durable decision wins. Later exit evidence is
retained without replacing Cancellation or Timeout. Stop reasons are separate
from results. Cancelling before the start call revokes the permit and confirms
that no process was launched by that session.

Polling stops remaining local processes after cancellation, timeout, main-process
exit, or recovery. It requests graceful stopping, then force stopping after the
retained cleanup grace. Stop acceptance, force calls, and missing units do not
prove cleanup. Uncertain stop replies require review. Slots and log reservations
remain held until cleanup and output handling finish. Cancellation and timeout
of launched work conservatively require review of possible external work.

Stops use the retained invocation and cgroup identity. Discovery by immutable run
ID supplies identity when none was retained. A changed identity is never adopted.
No SQLite transaction remains open across a supervisor call. Stops can repeat.
Starts cannot. This library does not release supervisor unit references. The
service must record cleanup, release the reference, then finish output capture.

### Output and storage failures

Output capture collects stdout/stderr into retained files. `output_failure`
consumes its early failure signal. Log size truncation is not a failure. A
non-storage output failure stops that run, records output failure separately,
and requires review. An earlier Success becomes Interruption. Cancellation,
Timeout, Failure, and Interruption remain unchanged. An undecided run becomes
Interruption. A later successful exit cannot clear the output failure.

A storage failure sets the shared irreversible `safety::Safety` gate. Connect
all admission and dispatch handles with `open_with_safety` and clones of the
same gate. New admissions and starts close without another successful write.
Matching admitted retries on healthy connections retain their identity. A
failed dispatch evidence write also sets the gate. `storage_failure` handles
external storage faults such as capture setup failure. It needs no database
write. Polling stops all retained active runs even when the store is closed.
The gate does not clear on pause, resume, or reopening with the same gate.

After a failed write, in-memory observations do not establish durable results or
cleanup. Restart recovery requires review and never repeats a start. A service
must preserve available failure evidence and inspect output before claiming
completion. This library cannot reconstruct a failure whose write was lost.
The local service wires each store to the same gate.

`finish` requires confirmed cleanup and successful synchronized output.
`finish_failed_output` instead requires a recorded output failure and the caller's
confirmation that output workers have ended and partial evidence was handled.
It cannot claim synchronized lost bytes or Success. Both retain the review block,
check retained bytes against the reservation, and release the slot only on commit.
[Administrator acknowledgement](execution-internals.md#administrator-review-and-acknowledgement) can then clear that run’s review
block with one note and the unchanged reviewed evidence.
A storage-closed handle cannot complete a run.

### Timestamps and compatibility

Schema version 4 introduced active Cancellation and Timeout, stop reason, output-failure
state, and nullable UTC millisecond dispatch/completion timestamps. `times` reads
them. Completion time is the commit of cleanup and output handling, including
queued cancellation. It is not program exit time. Clock adjustments can make
wall-clock timestamps non-increasing. Never use them for timeout decisions.
Migration leaves historical timestamps unknown and preserves sequence high-water
marks, permanent admissions, and all stored identity and revision bytes.

## Verification

Synthetic tests cover exclusive ownership, FIFO and capacity, cancellation races,
crashes on both sides of launch intent and supervisor acceptance, rollback,
uncertain replies and identity, timeout boundaries, graceful and force stopping,
and restart without a second start. They also cover log reclamation under
capacity pressure, partial deletion, and database rollback. Review
[isolated-host evidence](acceptance-evidence.md) separately.
