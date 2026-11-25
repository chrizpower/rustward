# Admission, review, and supervision internals

This reference describes the implemented Rust libraries, storage encodings,
and systemd adapter. Keep it while these details guide integration or
verification. Transfer any lasting decisions to [architecture](../ARCHITECTURE.md)
and preserve compatibility checks in code and tests before removal.

## Manual admission library

This library implements manual admission and queued cancellation. Dispatch has a
[separate session interface](execution.md).
The caller selects an opaque prepared runtime identity; admission retains it but
does not verify runtime files. [Prepared verification](runtime.md) is
a separate dispatch gate. See the [restore contract](operations.md#application-state-restore-contract).

### Storage and transactions

`admission::Store::open` opens a dedicated local SQLite database. Empty paths, `:memory:`, and SQLite URI handling are excluded. Schema version
7 uses rollback journaling and `synchronous=FULL`. An immediate transaction
serializes request lookup, queue capacity checks, and insertion of both a queued
run and its permanent admission record. A response is returned only after commit.
Run IDs are random 256-bit hexadecimal strings. Queue order is the insertion
sequence. The default capacity is 100; limits are stored in the database and can
be changed through `set_queue_limit`. Lowering the limit preserves existing runs.

Queued runs retain the normalized task snapshot, configuration revision,
generated arguments, submitted inputs, and runtime identity. The
[configuration reference](configuration.md#normalization-and-revisions) owns
snapshot normalization and revision encoding.

Storage errors close the affected store handle. The service must close all
admission and dispatch paths on such an error under P06. The controller connects
the shared Safety gate and emergency stopping. Lock contention returns a separate
busy response after a five-second wait. Reopening a failed handle is an explicit
caller action, not automatic recovery. Use a protected database directory: issuer
secrets and non-secret submitted inputs are stored there. No rejection history
is stored. Diagnostics use fixed messages and omit payloads and SQLite details.

### Request identity

Keys are lowercase hex encodings of 97 bytes: format byte 1, 32-byte generation,
32-byte random nonce, and 32-byte HMAC-SHA-256 over the preceding bytes. Each
installation database retains its random generation and 32-byte signing secret.
Issuance writes no key record and reserves no capacity. OS randomness supplies
secrets, generations, nonces, and run IDs. HMAC verification uses the library's
constant-time verification. Known admitted keys are looked up before current
issuer verification, allowing future restore generations to retain known keys.
There is no restore command in this slice.

Request fingerprint version 1 hashes `rustward-request-v1`, then length-prefixed
UTF-8 task ID, input count, and entries sorted by input ID. Lengths and counts
are unsigned 64-bit big-endian values. Each entry contains a length-prefixed ID,
then a type byte (`s`, `i`, or `b`) and value. Strings use length-prefixed UTF-8;
integers use signed 64-bit big-endian bytes; booleans use byte 0 or 1. Defaults
are excluded. Unsupported scalar types and composite values are rejected.
An omitted value thus differs from an explicit default.

A matching admitted key returns the original run ID before consulting the
current task, runtime selection, admission control, or queue capacity. A changed
payload is rejected. A missing history row returns `HistoryRemoved`; it does not
create a run. A fresh execution uses a fresh key. A rejected request leaves the key unbound,
so a corrected request can use it. Keep the fingerprint and original run ID
indefinitely under the P07 exception. A retry discloses no result after history
removal. Scheduled occurrences use a separate identity.

Requests are limited to 256 inputs and 64 KiB of canonical request bytes; runtime
identities are nonempty and at most 1024 bytes. Task snapshots and generated
arguments together are limited to 1 MiB per admission. These are implementation
bounds for this library, not transport or global metadata limits. Admission can
be closed independently through a stored control.

### Admission verification

Tests cover concurrent retries, queue capacity, payload conflicts, corrected
rejections, immutable snapshots, removed tasks and history, issuer changes,
failed transactions, and recovery of an uncommitted write. Host power-loss
durability remains unverified.

## Queued cancellation library

`admission::Store::cancel_queued(run_id)` completes a queued run with the result
`Cancellation`. It does not start a process or perform local process cleanup:
this run has never dispatched. The method works while admission is closed.
`Cancellation::Cancelled` is returned only after the transaction commits.
A repeated cancellation returns `AlreadyCancelled` without changing the record.
A missing run returns `NotFound`, including when only its permanent admission
record remains. A dispatched run returns `NotQueued`. No history is recreated.

Cancellation uses an immediate transaction to read the run state and update
phase and result together. Admission uses the same SQLite write lock. Queue
capacity counts only queued rows, so a committed cancellation releases capacity
without deleting history. Competing cancellations produce one change. A
concurrent admission sees capacity before or after the committed cancellation;
a queue-full response can be retried with its unbound key. A failed transaction
leaves the run queued and uses the existing storage-error handling.

`Store::run_state(run_id)` returns separate phase and result fields. Invalid
stored combinations close the handle with a storage error. Schema version 7 adds
controller output/setup evidence to [review](#administrator-review-and-acknowledgement).

Migration from versions 1–6 preserves run data, results, issuer state,
admission records, revisions, and sequence high-water marks. Version 1 first gains
the version 2 cancellation model. All steps commit together. Failed migrations
roll back; older binaries reject version 7. Tests retain SQL fixtures and
check failed table replacement, preserved identities, and sequence order.

## Administrator review and acknowledgement

The user accepted one note for one run and local completion before
acknowledgement on 2026-11-09. The [local control interface](service.md#local-service)
exposes review and acknowledgement.

### Meaning and record

Acknowledgement is the administrator's decision that this run's external
uncertainty need not block new starts. It can accept remaining risk. It does not
prove external completion, change the run result, or retry work.

The administrator supplies one nonempty note. It explains what was checked,
what remains uncertain, and why new starts are permitted. There are no separate
evidence, uncertainty, reason, or Resolved fields. The library cannot validate
the meaning of prose. Notes preserve their exact text, must contain non-whitespace text,
contain no NUL, and fit 4096 UTF-8 bytes.

Rustward automatically retains the run ID, reviewed evidence, and UTC millisecond
acknowledgement time. The evidence includes retained task/runtime IDs and
revisions, phase/result, launch/exit/cleanup evidence, output-failure state, log
accounting, and dispatch/completion times. It copies stored metadata, not task
output, credentials, or whole task definitions. The note can describe external
checks. The record cannot reconstruct transient evidence that was never stored.

### Reading and acknowledging

Methods are on `admission::Store`, so an administrator connection can operate
while the exclusive dispatch session remains open:

- `review(run_id)` returns one consistent `review::Review`, or None when there is
  no pending review for that run.
- `pending_reviews(after, limit)` returns pending reviews in run ID order. The
  page limit is 1–100 entries. Use the last returned ID as `after`; an empty
  page ends the scan. Pages are separate snapshots and do not authorize release.
- `review::new_acknowledgement_id()` supplies a random 64-character lowercase hex
  acknowledgement ID. It is not an admission key or authorization credential.
- `acknowledge(acknowledgement_id, &review, note)` retains the record and clears only that
  run's review block in one immediate SQLite transaction.
- `acknowledgement(acknowledgement_id)` retrieves the retained acknowledgement.

Keep the returned Review unchanged when submitting it. A new acknowledgement must match
both its evidence version and all copied evidence. `ReviewChanged` requires a
fresh read. `NotCompleted` means local cleanup and output handling have not
finished; acknowledgement requires Completed and confirmed local cleanup.
An unknown or no-longer-pending run cannot be acknowledged with a new acknowledgement ID.

There is one run per acknowledgement. Results, first exit evidence, identities,
revision bytes, timestamps, and admitted request fingerprints remain unchanged.
Other runs' review blocks, dispatch pause, execution/log capacity, admission
closure, and the shared storage-failure gate remain in effect. Acknowledgement
neither starts work nor frees a slot. Local completion already released its slot.

### Evidence changes and retries

Schema version 5 introduced a nonnegative integer evidence version. Updates to launch,
cleanup, review, output, accounting, or lifecycle evidence while review is pending
advance it. Repeated writes can advance the version even if displayed values
are equal. Recovery advances pending unfinished evidence. Overflow fails instead
of wrapping. Clearing review through acknowledgement does not advance it.
New uncertainty raises a later version; an old acknowledgement cannot clear it.

Exact retries with the same acknowledgement ID, Review, and note return the original
acknowledgement before current-state validation, including after local history
removal. A changed payload with the same acknowledgement ID returns PayloadConflict.
A competing new acknowledgement ID for an acknowledged review returns ReviewChanged.
Keep the acknowledgement ID and submitted Review until the response is known.

The stored record uses encoding 1. Acknowledgements have no deletion or editing
API. The copy preserves reviewed metadata after the controller
reclaims log capacity or future cleanup removes run history.

### Failure, compatibility, and verification

Acknowledgement insertion and review release commit together. Failed writes
roll back both changes and storage errors set the shared Safety gate.
Retry a busy transaction or lost reply with the same ID and payload.

Migration preserves existing review flags and gives pending legacy rows
evidence version 1. Version 6 adds optional `launch_failure`. Version 7 adds
`capture_setup_failure` and `output_evidence`. Encoding 1 omits absent optional
fields and keeps stored acknowledgements readable without a rewrite.

Tests cover completion, changed evidence, competing acknowledgements, matching
retries, other dispatch gates, rollback, restart, retained history, note/page
limits, and migration.

## Supervision interface and systemd adapter

The user selected a dedicated unprivileged systemd user manager and supplied
Debian 13 as the build baseline. The adapter targets systemd 257 or newer and
cgroup v2 on that host. It does not request system-manager or privileged unit
creation. Host setup must keep the dedicated user manager available. The
adapter process and manager must see the same cgroup hierarchy. Production host
setup remains in the deployment repository; nothing here authorizes deployment.

`supervision::Supervisor` defines start, observation, stop, and reference release.
Tests can replace the complete supervisor.
`systemd::Systemd<M, C>` implements this interface. Its `Manager` and `Cgroups`
ports permit controlled manager replies and cgroup evidence. `UserBus` implements
the manager port with zbus blocking D-Bus calls on the session bus, with no
system-bus fallback. Configure that session as the dedicated user's bus. Each
D-Bus method has a five-second timeout. `CgroupFs` reads at most 4096 bytes of
`cgroup.events` under `/sys/fs/cgroup`; malformed or unreadable evidence stays
unknown.

### Start and identity

A `Launch` uses an existing 64-character lowercase hexadecimal run ID, absolute
program and working-directory paths, separate arguments, and a cleanup grace
period. `DEFAULT_CLEANUP_GRACE` is 30 seconds; callers can supply another positive
period in whole microseconds. The adapter rejects NUL, more than 65,536 arguments,
and more than 1 MiB of combined program, directory, and argument text.

The unit name is `rustward-run-<run-id>.service`. `StartTransientUnit` uses job
mode `fail`. It never replaces an existing run unit or retries a failed call.
`StartOutcome::Accepted` confirms only that a start job was queued. Explicit
permission/argument errors report `Rejected`; an existing unit reports
`Conflict`. Other errors, including a lost reply, report `Uncertain`. None
permits an automatic new start for the same admitted run.

The adapter sets `Type=exec`, `Restart=no`, `KillMode=control-group`,
`SendSIGKILL=yes`, and the selected stop grace period. `ExecStartEx` includes
`no-env-expand`, preserving complete argument values without shell or environment
expansion. `RemainAfterExit=yes` retains a successful exited service for explicit
cleanup. AddRef pins the unit while its client lives; recovery uses RefUnit
after identity checks. Release needs durable cleanup.
Standard input is null; stdout and stderr use caller-supplied file descriptors.
The `capture` library supplies draining and per-run bounded retention. There is no implicit journal or
file-output fallback. Controlled starts use the pinned environment launcher and
drop privileges; custom environment and secrets remain future work. Legacy
low-level starts do not provide the controller environment.

An observed systemd invocation ID and cgroup path form
`supervision::ExecutionIdentity`. Dispatch retains this
correlation before using it for subsequent observation or stopping. This identity
is separate from the run ID and prepared runtime identity. The adapter checks
unit description, transient status, service type, and any supplied prior
invocation and cgroup. A mismatch prevents stop or release.

### Observation and cleanup

Up to three fresh snapshot pairs surround new cgroup reads. Only matching
snapshots provide evidence. Retain the first execution identity across pairs,
even if the caller supplied no prior identity. The user accepted the bounded
reread on 2026-11-11. Changed identity, lost reads, or three changed pairs
remain uncertain. A start job, PID, or process timestamp does not prove execution.
Running requires an active, running `Type=exec` unit with a main PID. Exit data
uses the manager's exit timestamp, exit code, and signal status. An exit code is
local evidence only; it does not prove an external outcome.

Cleanup is confirmed only when the unit is inactive or failed, no job is pending,
no main PID is reported, and its matching cgroup is empty or absent. A retained
prior identity permits checking the cgroup after systemd clears its current
cgroup path. Missing units, unknown cgroups, and missing PIDs alone do not prove
cleanup. Stopping is distinct from observing program exit. A graceful stop queues
`StopUnit`; force stopping sends SIGKILL to all unit processes. Neither response
confirms cleanup. `release` checks cleanup before calling `UnrefUnit`; callers
must first durably retain exit and cleanup evidence. Capture EOF and final
completion can follow reference release. Connection loss can
release references, so unit retention alone is not a recovery protocol.

### Verification

Synthetic manager, cgroup, and private D-Bus tests cover start settings, wire
signatures, descriptor transfer, uncertain replies, identity, exit, and cleanup.

The [isolated-host suite](acceptance.md) exercises child cleanup and concurrent
unit isolation, capture, and runtime permissions. The [controller](service.md#execution-controller)
uses the adapter with pinned runtime and host dependencies, UID binding, and
storage-failure stopping. Missing units and lost replies cannot cause another
start.

The implementation follows the [systemd 257 D-Bus interface](https://www.freedesktop.org/software/systemd/man/257/org.freedesktop.systemd1.html)
and its [transient launch property construction](https://github.com/systemd/systemd/blob/v257/src/run/run.c).
