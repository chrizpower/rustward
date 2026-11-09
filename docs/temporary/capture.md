# Bounded output capture

Status: Temporary reference for implemented per-run stdout/stderr capture.
Absorb the interface, bounds, failure handling, and verification limits into
ARCHITECTURE.md before removing this file. Preserve open integration work and
update inbound links. See [folder guidance](README.md). The rules are P06 and
P07 in [Gravity](../GRAVITY.md); the terms are in [the glossary](../GLOSSARY.md).

This library supplies output descriptors to the supervision adapter. It does
not dispatch tasks, reserve aggregate log space, remove retained logs, or change
run state. The [dispatch library](dispatch.md) provides conservative log accounting. Real
capacity integration, protected output, retention, runtime verification, and
service-wide failure handling remain open.

## Interface and files

`capture::Capture::create(root, &run_id, options)` creates a new directory named
with the admitted `supervision::RunId` under an existing protected log root. It
creates `stdout.log` and `stderr.log` with exclusive creation. Directory mode is
0700; file mode is 0600, subject to a more restrictive process umask. Existing
paths are never reused, truncated, or overwritten. A setup failure can leave
partial files for review; no automatic deletion or retry is provided.

Setup synchronizes the empty log files, new directory, and parent directory
before returning. Setup failures are `AlreadyExists`, `Storage`, or `Resources`.
Diagnostics omit paths, output, and underlying error messages. The caller must
stop setup before starting any program if capture cannot be created.

`Options::default()` sets a shared 10 MiB retained-byte limit. The caller can set
`retained_bytes` for each capture without changing TOML version 1. A zero limit
keeps no output and still drains both streams. Successful empty output is not
truncation, even at a zero limit. Filling the limit exactly is not truncation;
additional observed bytes establish truncation.

Creation returns a `Capture` and `OutputWriters`. `OutputWriters::output()`
borrows stdout and stderr descriptors in the existing `supervision::Output`
interface. The manager must duplicate those descriptors when accepting a start.
The caller must drop its local writers after the start call, including when
its response is rejected or uncertain. Remote copies can remain with the program and manager. A manager reference
can outlive its processes; callers must not assume that process exit alone
closes every writer.

The descriptors use Unix stream socket pairs. The two workers start before the
caller can launch a program. Each worker reads one stream with a fixed 8 KiB
buffer and writes raw bytes to its own file. There is no line buffering, text
decoding, event queue, or output-sized allocation. Kernel socket buffers are
finite and are separate from the two application read buffers. Active captures
must be bounded by the future dispatch limit; there is no global capture registry.

## Limit and stream ordering

Workers claim bytes from one atomic remaining allowance before writing. They
handle partial writes and interrupted I/O without exceeding the allowance. A
report counts only bytes actually written as retained. The sum of retained bytes
across both files cannot exceed the configured limit. Read counters saturate at
`u64::MAX` rather than overflowing.

Each file retains a prefix of its own stream. Scheduling determines which stream
uses the shared allowance first; the library promises no fairness or total order
between streams. A stream that loses bytes because the allowance is exhausted
is marked truncated. Truncation is normal limit handling. Both workers continue
draining to EOF after truncation, without more output writes. Slow storage can
cause backpressure before the limit; this library does not promise fixed drain
latency or protect against a storage call that never returns.

## Completion and failure

`Capture::report()` reads live, monotonic byte counters and stream truncation/EOF
flags. It is not a synchronized final snapshot. EOF means the output writer
closed, not that the program exited, local cleanup completed, or log storage was
synchronized. Capturing output does not prove an external outcome.

`Capture::failure()` reports the first failure through shared atomic state,
without waiting for EOF or taking a storage lock. The signal has fixed size and
cannot grow with output. It remains set even when later operations succeed.
Failures distinguish stream read, log write, flush, synchronization, worker
panic, and abandonment. `Failure::is_storage()` identifies write, flush, and
synchronization faults. A storage fault is not converted to truncation.

After a failure, workers stop starting output writes and continue draining
readable streams. An already pending write can finish. A failed read ends that
stream's worker without asserting EOF. A worker panic is caught and reported;
its reader closes. On stream completion or a read error, each worker flushes
and synchronizes its retained prefix. These operations cannot clear an earlier
failure. Partial successful writes remain counted as retained evidence.

`Capture::finish()` waits for both workers and returns a final `Report` only
when both streams ended and the log operations succeeded. On failure, it returns
`CaptureError` with the first failure and partial report. All local and remote
output writers must close before normal completion can finish. For the systemd
adapter, first confirm cleanup and durably retain exit/cleanup evidence, then
release the unit reference before waiting for capture EOF. Record final run
completion only after successful capture synchronization. Callers must
check failure while work is active rather than waiting for `finish()` to detect
a fault that requires stopping the program. Reports and failure signals are
in-memory results; the future dispatcher must durably record them before
claiming completed execution.

Under P06, the future service must close admission and dispatch and stop all
active local work on storage failure. It must preserve uncertainty for review.
`SetupError::Storage` also invokes P06 while other work is active. This library
exposes failures without requiring a database write; it does
not implement those service-wide controls. The [stopping library](stopping.md)
consumes failure signals and shares a storage-failure gate; a service must connect
all handles and continue polling to perform stops. Read and worker failures also mean
capture is incomplete; they must not support a successful execution result.

Dropping an unfinished `Capture` marks it abandoned, shuts down both readers,
and joins the workers. Producers can then receive a broken-pipe error. This
releases idle readers without detached drain threads, but does not stop a
systemd unit or confirm cleanup. Drop can wait for a storage operation already
in progress. Abandonment provides no durable-success claim and is not a
substitute for supervising and stopping active work.

## Verification and remaining work

Tests use synthetic bytes, Unix descriptors, temporary files, and injected I/O
failures. They cover separate binary prefixes, shared limits, zero and exact
limits, concurrent output flooding, continued draining after truncation and
write failure, partial writes, interrupted reads/writes, read/flush/sync errors,
worker panic, early failure reporting, protected file creation, and abandonment.
A `/dev/full` test checks native file-write failure. A fake supervisor duplicates
the descriptors and writes output without a manager.

These tests do not prove real systemd capture, filesystem behavior after power
loss, aggregate log bounds, retained review protection, or service-wide P06
handling. An opt-in Debian 13 host test connects these descriptors to the real adapter,
checks both streams and the shared limit, and stops/releases the unit before
waiting for EOF. It and the earlier manager cleanup test remain unrun here. There is no transport/API or live output stream to a UI in this slice.
