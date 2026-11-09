# Active stopping and failure handling

Status: Implemented library, tested with fake supervision. The user accepted
output-failure rules and a one-hour default timeout on 2026-11-08. This does not
authorize real execution. Transfer stable interfaces to ARCHITECTURE.md and
policy to GRAVITY.md before removal. Preserve open integration work.

## Task settings and retained data

Version 2 task files accept `timeout_seconds` (default 3600) and
`cleanup_grace_seconds` (default 30). Both are whole seconds from 1 through
4294967295; zero and disabled timeouts are not supported for new definitions.
New admissions retain both values in normalized task snapshots. Explicit defaults
and omitted defaults have equal revisions. Timing edits affect new admissions.
Stored task IDs, snapshots, revisions, fingerprints, and matching retries remain
unchanged. Legacy snapshots without these fields retain no timeout and a
30-second cleanup grace. No history is rewritten.

The timeout starts at dispatch and includes launch. A monotonic `Instant` measures
execution and cleanup time; UTC wall-clock changes do not change deadlines.
`Session::poll(supervisor, now)` requires explicit calls with monotonic time.
There is no background timer or daemon. A service must poll promptly, including
after failures. Polling delay can delay stopping. Recovery never resets a timeout
or starts recovered work: it retains a decided result or records Interruption,
requires review, and begins cleanup.

## Decisions and stopping

`cancel_active` records Cancellation for an unfinished dispatched run unless a
result already exists. It returns Recorded, AlreadyEnded, or NotActive. Queued
cancellation remains `Store::cancel_queued`. Timeout, exit, and cancellation use
SQLite transactions; the first durable decision wins. Later exit evidence is
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
No SQLite transaction remains open across a supervisor call. Stops can repeat;
starts cannot. This library does not release supervisor unit references. The
service must record cleanup, release the reference, then finish output capture.

## Output and storage failures

Output capture collects stdout/stderr into retained files. `output_failure`
consumes its early failure signal. Log size truncation is not a failure. A
non-storage output failure stops that run, records output failure separately,
and requires review. An earlier Success becomes Interruption; Cancellation,
Timeout, Failure, and Interruption remain unchanged. An undecided run becomes
Interruption. A later successful exit cannot clear the output failure.

A storage failure sets the shared irreversible `safety::Safety` gate. Connect
all admission and dispatch handles with `open_with_safety` and clones of the
same gate. New admissions and starts close without another successful write;
matching admitted retries on healthy connections retain their identity. A
failed dispatch evidence write also sets the gate. `storage_failure` handles
external storage faults such as capture setup failure. It needs no database
write. Polling stops all retained active runs even when the store is closed.
The gate does not clear on pause, resume, or reopening with the same gate.

After a failed write, in-memory observations do not establish durable results or
cleanup. Restart recovery requires review and never repeats a start. A service
must preserve available failure evidence and inspect output before claiming
completion; this library cannot reconstruct a failure whose write was lost.
Independent default `open` calls have independent gates and do not establish
service-wide failure handling. Service wiring remains required.

`finish` requires confirmed cleanup and successful synchronized output.
`finish_failed_output` instead requires a recorded output failure and the caller's
confirmation that output workers have ended and partial evidence was handled.
It cannot claim synchronized lost bytes or Success. Both retain the review block,
check retained bytes against the reservation, and release the slot only on commit.
[Administrator acknowledgement](review.md) can then clear that run’s review
block with one note and the unchanged reviewed evidence.
A storage-closed handle cannot complete a run.

## Timestamps and compatibility

Schema version 4 introduced active Cancellation and Timeout, stop reason, output-failure
state, and nullable UTC millisecond dispatch/completion timestamps. `times` reads
them. Completion time is the commit of cleanup and output handling, including
queued cancellation; it is not program exit time. Clock adjustments can make
wall-clock timestamps non-increasing; never use them for timeout decisions.
Migration leaves historical timestamps unknown and preserves sequence high-water
marks, permanent admissions, and all stored identity and revision bytes.

## Evidence and remaining gates

Synthetic tests cover decision ordering, timeout boundaries, retained task timing,
pre-start cancellation, graceful/force stopping, cleanup/output completion gates,
failed writes, shared admission closure, and recovery without another start.
Version 3 migration tests preserve active execution identity and unknown times.

Runtime/manager UID binding, host dependency policy, automatic capture orchestration, actual supervisor
observation and release, retention reclamation,
and the service loop remain unimplemented. Isolated Debian 13/systemd 257 host
checks remain required before real execution. No scheduler, live reload,
production migration, or deployment is included.
