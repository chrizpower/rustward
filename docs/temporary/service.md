# Local service and controller

`rustward serve` starts the local service. `rustward ctl` sends one command to
that service. The service runs the controller with the dedicated systemd user
manager. See [acceptance evidence](acceptance-evidence.md) for the scope of
recorded host passes.

## Local service

Run as the dedicated non-root execution user with access to its authenticated
systemd user bus. Supply absolute catalog and work paths. Provision the state
database's parent, log root, and work root as private directories owned by
the execution user. Use separate prepared runtime and host catalogs. Select the
runtime ID and preparation UID explicitly at startup.

```sh
rustward serve rustward.toml main /path/to/runtimes /path/to/hosts /path/to/work "$runtime_id" "$preparation_uid"
printf '%s\n' '{"command":"service_status"}' | rustward ctl main
printf '%s\n' '{"command":"resume"}' | rustward ctl main
```

The service validates the complete configuration once and verifies the selected
runtime and its pinned host manifest. It applies the configured queue,
execution, and log limits. It acquires dispatch ownership, recovers unfinished
runs, and starts paused on each invocation. Admission accepts new requests once
startup succeeds. Review still blocks dispatch after an explicit resume.
Each run retains its admitted runtime ID, which the controller verifies again
before launch.

`ctl` reads one JSON request from standard input and prints one JSON response. The client closes its write side after sending the request.
The Linux abstract Unix socket is scoped to the service name and effective UID.
Both ends check peer credentials. Use the same execution UID and network
namespace for the service and client. Names use 1–64 ASCII letters, digits,
hyphens, or underscores. Requests and responses are limited to 128 KiB.
The service allows 250 ms per socket read or write phase. The client sets
a 30-second read timeout.

| Command | Additional fields |
| --- | --- |
| `issue_key` | None |
| `submit` | `key`, `task_id`, and optional `inputs` object |
| `status`, `cancel` | `run_id` |
| `service_status`, `pause`, `resume`, `shutdown` | None |
| `reviews` | Optional `after` run ID. Returns up to ten reviews in run ID order. |
| `acknowledge` | `acknowledgement_id`, exact `review` object, and `note` |

A response contains `ok` with the result or `error` with an error name. A request
failure returns a nonzero exit status. Retain the key and inputs to retry a
submission whose response is lost. The service reports dispatch blocks through
`service_status`. Use the acknowledgement ID, review object, and note again
if an acknowledgement reply is lost.

SIGTERM, SIGINT, and the `shutdown` command stop new admission and dispatch.
The service continues polling active runs to durable completion. A storage
failure activates the shared safety gate and emergency stopping. An abrupt
process exit uses the existing recovery and review protocol on restart.

The isolated-host runner includes a daemon case that submits through `ctl`,
checks matching retries and log reclamation, kills and restarts the daemon,
checks review and protected output, and checks planned shutdown with active
and queued work. It writes `evidence/daemon.json` after its assertions pass.

## Execution controller

### Binding and interface

`controller::Controller::open(database, Roots, dispatch::Limits, supervisor, safety)`
opens the exclusive dispatch session. Roots contains a runtime catalog, a host
catalog, and separate private WorkRoot values for writable run data and logs.
All four paths must be separate, with no ancestor overlap. The actual service
UID, both catalogs' execution UIDs, both private roots' owner UIDs, and the
manager's authenticated UID must match. UID 0 is rejected. The service must have
zero effective, permitted, inherited, and ambient Linux capabilities. The
runtime and host preparation UID must differ from the execution UID.

Use the same cloned Safety for admission Store handles and the controller.
Independent default Store handles do not provide service-wide failure control.
The constructor reads paths and authenticates the manager. It does not admit or
start a task. It recovers unfinished dispatched records conservatively.

`start_next` claims the oldest queued run under the existing pause, review,
capacity, and storage gates. It returns Preparing(run ID) or Blocked(reason).
A bounded preparation worker holds the one-use permit. The controller does not
start work automatically or resume dispatch during construction. `set_paused`
changes only pause. `tick(Instant)` records timeout and supervision evidence,
starts prepared work at most once, and advances output handling and completion. If a review gate
closes before a prepared run starts, it records Interruption, review, and known
no-start cleanup. An earlier cancellation or timeout stays unchanged.
Call tick regularly, including after a stop or storage failure. Supplied times
must come from the same monotonic clock. `cancel_active`, `run_state`, `record`,
`active_runs`, and `failed` expose existing controls and state. Queued admission,
queued cancellation, and completed-run acknowledgement stay on Store.

A slot remains reserved while preparation, cleanup, manager release, or output
handling is pending. Cancellation or timeout during preparation prevents launch.
It does not release the slot while the worker still holds resources. Preparation
and capture setup failures consume the permit and retain a fixed failure code.
Schema version 7 adds optional capture_setup_failure and output_evidence to the
launch record. Completion saves final output evidence and retained-byte
accounting in the same transaction. Failures before capture exists keep output
evidence absent. Failed writes cannot publish completion.

### Controlled launch

The UserBus authenticates the manager through D-Bus GetNameOwner and
GetConnectionUnixUser. It pins the unique bus owner before reading the manager
baseline. Controlled starts recheck that owner and UID. A replacement or
mismatch cannot silently select another manager. The emergency adapter clone
uses the pinned owner. A service restart must use the same dedicated manager and
execution user to inspect retained units.

Controlled transient units use NoNewPrivileges=true, CapabilityBoundingSet=0,
and AmbientCapabilities=0, in addition to the existing Type=exec, no-restart,
control-group cleanup, and no-env-expand rules. The manager's environment names
and fixed loader/locale allocator controls are removed with UnsetEnvironment.
The dedicated manager environment must remain stable and free of injection
settings during launch. This protects launcher startup. A concurrent trusted
administrator changing manager state is outside the atomicity guarantee.
See [systemd 257 execution settings](https://www.freedesktop.org/software/systemd/man/257/systemd.exec.html).

Build `rustward-exec` with the project and publish it under the pinned launcher
path. The private positional protocol is:

```text
rustward-exec <runtime-directory> <writable-directory> <executable> [arguments...]
```

The helper clears inherited variables, sets exactly these seven entries, and
replaces itself with the executable. There is no shell, argument expansion,
configuration lookup, or second task start. The original executable, working
directory, and argument vector retain their admitted meanings, including empty
arguments, spaces, metacharacters, and `=` in paths. Runtime catalog paths cannot
contain `:` because PATH uses that separator. Profile paths are limited to
4096 bytes each. Supervisor clones must allow concurrent emergency calls.

| Variable | Value |
| --- | --- |
| PATH | Runtime directory followed by `/bin` |
| HOME | Writable run directory |
| TMPDIR | Writable run directory |
| RUSTWARD_WORK_DIRECTORY | Writable run directory |
| LANG | C.UTF-8 |
| LC_ALL | C.UTF-8 |
| TZ | UTC |

The configured working_directory stays inside the selected runtime. It is not
replaced by HOME. Custom environment variables and secret references are not
implemented. The helper is an execution mechanism, not a service CLI or a
security boundary between tasks that share the execution UID.

### Stopping, output, and recovery

An independent emergency thread reads atomic capture monitors and the shared
Safety gate. It uses retained run/invocation/cgroup identity to request graceful
stop, then force after the retained cleanup grace. It needs no database write,
file hashing, capture join, or log synchronization. A non-storage capture fault
stops that run. Any observed capture storage fault, including one after an
earlier read failure, closes admission/dispatch through Safety and stops all
active called runs. Capture setup storage failure uses the same gate. Manager
calls still have transport timeouts. This is not a fixed stop-latency guarantee.

The controller records capture failure separately. It preserves cancellation,
timeout, failure, and interruption. A prior Success becomes Interruption when
output evidence is incomplete. Review stays required. Missing units do not
prove cleanup. Cleanup proof is durably saved before release of the manager's
unit reference. Only then can normal output EOF and worker completion release
capacity. EOF alone cannot finish a run. Capture completion is checked without
waiting on live workers or file synchronization.

Recovery never creates a permit or starts an unfinished dispatched run again.
Before stopping, it calls Supervisor::reference once per recovered run. The native
adapter checks the retained invocation/cgroup identity, acquires RefUnit, and
checks identity again. This holds the existing unit's evidence after the old
client died. Failed acquisition retains uncertainty. It never proves cleanup.
The reference is released only through the existing cleanup/release protocol.
It preserves stored identities and results, stops retained uncertain work, and
requires review. After cleanup proof and manager release, an I/O worker checks
existing private log files and synchronizes their retained prefixes. Historical
observed counts and truncation are unknown, not inferred from file lengths.
Recovered unfinished output is marked failed even when current synchronization
succeeds. It cannot support Success. Missing/unsafe files retain partial failed
evidence. Oversized output cannot complete against a smaller reservation.
Storage-closed handles cannot assert durable cleanup or completion.

Drop requests emergency stopping but does not prove cleanup or drain an active
run to completion. It can wait on capture I/O. Planned service shutdown must stop
new dispatch and continue the controller until active runs finish, as required
by the existing contract. The local service drives this drain.

## Bounded output capture

### Interface and files

`capture::Capture::create(root, &run_id, options)` creates a new directory named
with the admitted `supervision::RunId` under an existing protected log root. It
creates `stdout.log` and `stderr.log` with exclusive creation. Directory mode is
0700. File mode is 0600, subject to a more restrictive process umask. Existing
paths are never reused, truncated, or overwritten. A setup failure can leave
partial files for review. No automatic deletion or retry is provided.

Setup synchronizes the empty log files, new directory, and parent directory
before returning. Setup failures are `AlreadyExists`, `Storage`, or `Resources`.
Diagnostics omit paths, output, and underlying error messages. The caller must
stop setup before starting any program if capture cannot be created.

`Options::default()` sets a shared 10 MiB retained-byte limit. The caller can set
`retained_bytes` for each capture without changing service TOML version 2. A zero limit
keeps no output and still drains both streams. Successful empty output is not
truncation, even at a zero limit. Filling the limit exactly is not truncation.
Additional observed bytes establish truncation.

Creation returns a `Capture` and `OutputWriters`. `OutputWriters::output()`
borrows stdout and stderr descriptors in the existing `supervision::Output`
interface. The manager must duplicate those descriptors when accepting a start.
The caller must drop its local writers after the start call, including when
its response is rejected or uncertain. Remote copies can remain with the program and manager. A manager reference
can outlive its processes. Callers must not assume that process exit alone
closes every writer.

The descriptors use Unix stream socket pairs. The two workers start before the
caller can launch a program. Each worker reads one stream with a fixed 8 KiB
buffer and writes raw bytes to its own file. There is no line buffering, text
decoding, event queue, or output-sized allocation. Kernel socket buffers are
finite and are separate from the two application read buffers. Active captures
must be bounded by dispatch capacity. The controller owns a bounded active-run registry.

### Limit and stream ordering

Workers claim bytes from one atomic remaining allowance before writing. They
handle partial writes and interrupted I/O without exceeding the allowance. A
report counts only bytes actually written as retained. The sum of retained bytes
across both files cannot exceed the configured limit. Read counters saturate at
`u64::MAX` rather than overflowing.

Each file retains a prefix of its own stream. Scheduling determines which stream
uses the shared allowance first. The library promises no fairness or total order
between streams. A stream that loses bytes because the allowance is exhausted
is marked truncated. Truncation is normal limit handling. Both workers continue
draining to EOF after truncation, without more output writes. Slow storage can
cause backpressure before the limit. This library does not promise fixed drain
latency or protect against a storage call that never returns.

### Completion and failure

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
stream's worker without asserting EOF. A worker panic is caught and reported.
Its reader closes. On stream completion or a read error, each worker flushes
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
a fault that requires stopping the program. The controller records final
Evidence and retained-byte accounting atomically with run completion.

Under P06, the service must close admission and dispatch and stop all
active local work on storage failure. It must preserve uncertainty for review.
`SetupError::Storage` also invokes P06 while other work is active. This library
exposes failures without requiring a database write. It does
not implement those controls alone. The [controller](#execution-controller) connects monitors to an independent emergency stop worker and the shared Safety gate. Read and worker failures also mean
capture is incomplete. They must not support a successful execution result.

Dropping an unfinished `Capture` marks it abandoned, shuts down both readers,
and joins the workers. Producers can then receive a broken-pipe error. This
releases idle readers without detached drain threads, but does not stop a
systemd unit or confirm cleanup. Drop can wait for a storage operation already
in progress. Abandonment provides no durable-success claim and is not a
substitute for supervising and stopping active work.

### Nonblocking observation and recovery

`Capture::monitor()` returns a cloned atomic Monitor. It reports the first
failure, live counters, and a sticky storage_failed signal. A later storage
fault remains visible even if a read or worker failure was recorded first.
`try_finish` returns None while a worker is active, including during log sync.
It never waits on active I/O. Once all workers end it returns the final report
or failure. The controller uses this method instead of joining live workers.

After durable cleanup and manager release, `capture::recover` checks existing
private per-run files without following symlinks and synchronizes retained
prefixes. It rejects unsafe modes, ownership, hard links, and oversized output.
Recovered Evidence keeps retained lengths but leaves historical observed counts
and truncation unknown. The controller marks unfinished recovered output failed.
Current synchronization cannot establish complete capture before a crash.

### Verification

Synthetic tests exercise the service through its control interface, controller
ordering and failure paths with synthetic supervisors, and output capture with
real Unix descriptors and injected I/O faults. The [isolated-host suite](acceptance.md)
also checks the native systemd adapter, runtime permissions, and capture.
