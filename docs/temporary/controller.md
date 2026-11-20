# Execution controller

Status: Implemented library, with synthetic tests. The user accepted one non-root
execution user, a clean environment, and pinned host manifests on 2026-11-09.
No real controller run or Ansible acceptance case has passed. This work does not
authorize live tasks, migration, or deployment.

Before removal, transfer stable interfaces to ARCHITECTURE.md, accepted policy to
GRAVITY.md, and shared meanings to GLOSSARY.md. Preserve host acceptance gates,
remaining work, evidence, and inbound links.

## Binding and interface

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
The constructor reads paths and authenticates the manager; it does not admit or
start a task. It recovers unfinished dispatched records conservatively.

`start_next` claims the oldest queued run under the existing pause, review,
capacity, and storage gates. It returns Preparing(run ID) or Blocked(reason).
A bounded preparation worker holds the one-use permit. The controller does not
start work automatically or resume dispatch during construction. `set_paused`
changes only pause. `tick(Instant)` records timeout and supervision evidence,
starts prepared work at most once, and advances output handling and completion. If a review gate
closes before a prepared run starts, it records Interruption, review, and known
no-start cleanup. An earlier cancellation or timeout stays unchanged.
Call tick regularly, including after a stop or storage failure; supplied times
must come from the same monotonic clock. `cancel_active`, `run_state`, `record`,
`active_runs`, and `failed` expose existing controls and state. Queued admission,
queued cancellation, and completed-run acknowledgement stay on Store.

A slot remains reserved while preparation, cleanup, manager release, or output
handling is pending. Cancellation or timeout during preparation prevents launch;
it does not release the slot while the worker still holds resources. Preparation
and capture setup failures consume the permit and retain a fixed failure code.
Schema version 7 adds optional capture_setup_failure and output_evidence to the
launch record. Completion saves final output evidence and retained-byte
accounting in the same transaction. Failures before capture exists keep output
evidence absent. Failed writes cannot publish completion.

## Runtime and host dependencies

Runtime version 1 manifests can include `host = "<host-id>"`. It participates in
the runtime hash. When absent, serialization omits it, so existing runtime IDs
stay unchanged. The low-level runtime verifier still accepts old manifests;
the controller requires a host reference. It never chooses another runtime or
host when verification fails. Admission snapshots and permanent request
fingerprints are unchanged. Catalog relocation does not rewrite admitted paths
or history. A moved command requires a new definition for future admissions.

A host catalog contains `<host-id>.toml`. Example schema:

```toml
version = 1
launcher = "/opt/rustward/rustward-exec"

[files."/opt/rustward/rustward-exec"]
sha256 = "<64 lowercase hexadecimal characters>"
executable = true

[files."/usr/lib/example/libexample.so"]
sha256 = "<64 lowercase hexadecimal characters>"

[links]
"/lib" = "usr/lib"
```

This is an illustrative schema, not a usable manifest. Pin actual hashes for the
Rustward launcher, its loader and libraries, task interpreters, and every other
host dependency used outside the runtime. The launcher must be a pinned
executable that implements the protocol below. A file's executable flag defaults
to false and must agree with its modes. Unknown fields are rejected.

`runtime::host::manifest_id` hashes `rustward-host-v1` followed by sorted-key
normalized JSON. All manifest fields, absolute file paths, hashes, executable
flags, launcher path, and link targets participate. Catalog location does not.
Limits reuse runtime::Limits: by default 16 MiB manifest, 100,000 entries including
the manifest, 4 GiB declared content, 64 path components, and 4096 path bytes.

Host file paths are normal absolute UTF-8 paths. Manifest files and catalog
ancestors cannot use symlinks. Host dependencies can use explicitly pinned
symlinks because Linux loader and interpreter paths commonly use them. Each
link records its exact raw target. Resolution uses only declared links, rejects
cycles and more than 32 links, and must lead to declared content. A directory
alias can reach that content through another declared child link. Undeclared
links, special files, unsafe modes, and capabilities are rejected. Declared files,
links, and their ancestors must be protected from the execution user. Host
files/links can belong to root or the preparation UID. Catalog files belong to
the preparation UID. Runtime trees retain their stricter no-symlink rule.

Verification hashes declared files, checks ownership and modes, rechecks links,
checks observed file replacement/change, and rereads the manifest and catalog.
The controller verifies runtime and host before setup and again after capture
setup. All hashing and setup run outside the main polling and emergency paths.
The protected log root and writable directory are also checked before use.

The preparer must supply complete dependencies and the correct launcher. Hashes
do not establish a binary's behavior or discover arbitrary imports, subprocesses,
or network dependencies. Published runtime trees must remain fixed while queued
or active runs reference them. Host dependencies must remain fixed while active
runs use them; an operating-system update must wait for those runs. A queued run
whose host selection no longer matches fails before launch. Verification is not
an atomic snapshot against a trusted preparer that changes files concurrently.
No preparation, freeze enforcement, deletion, or deployment tool is supplied.

## Controlled launch

The UserBus authenticates the manager through D-Bus GetNameOwner and
GetConnectionUnixUser. It pins the unique bus owner before reading the manager
baseline. Controlled starts recheck that owner and UID; a replacement or
mismatch cannot silently select another manager. The emergency adapter clone
uses the pinned owner. A service restart must use the same dedicated manager and
execution user to inspect retained units.

Controlled transient units use NoNewPrivileges=true, CapabilityBoundingSet=0,
and AmbientCapabilities=0, in addition to the existing Type=exec, no-restart,
control-group cleanup, and no-env-expand rules. The manager's environment names
and fixed loader/locale allocator controls are removed with UnsetEnvironment.
The dedicated manager environment must remain stable and free of injection
settings during launch. This protects launcher startup; a concurrent trusted
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

The configured working_directory stays inside the selected runtime; it is not
replaced by HOME. Custom environment variables and secret references are not
implemented. The helper is an execution mechanism, not a service CLI or a
security boundary between tasks that share the execution UID.

## Stopping, output, and recovery

An independent emergency thread reads atomic capture monitors and the shared
Safety gate. It uses retained run/invocation/cgroup identity to request graceful
stop, then force after the retained cleanup grace. It needs no database write,
file hashing, capture join, or log synchronization. A non-storage capture fault
stops that run. Any observed capture storage fault, including one after an
earlier read failure, closes admission/dispatch through Safety and stops all
active called runs. Capture setup storage failure uses the same gate. Manager
calls still have transport timeouts; this is not a fixed stop-latency guarantee.

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
client died. Failed acquisition retains uncertainty; it never proves cleanup.
The reference is released only through the existing cleanup/release protocol.
It preserves stored identities and results, stops retained uncertain work, and
requires review. After cleanup proof and manager release, an I/O worker checks
existing private log files and synchronizes their retained prefixes. Historical
observed counts and truncation are unknown, not inferred from file lengths.
Recovered unfinished output is marked failed even when current synchronization
succeeds; it cannot support Success. Missing/unsafe files retain partial failed
evidence. Oversized output cannot complete against a smaller reservation.
Storage-closed handles cannot assert durable cleanup or completion.

Drop requests emergency stopping but does not prove cleanup or drain an active
run to completion. It can wait on capture I/O. Planned service shutdown must stop
new dispatch and continue the controller until active runs finish, as required
by the existing contract. The local service drives this drain.

## Evidence and remaining work

Synthetic tests cover FIFO/capacity, unchanged arguments, output truncation,
EOF before exit, cleanup/release/output ordering, cancellation, timeout,
preparation failure, lost start replies, rejection, missing units, recovery,
manager UID changes, private D-Bus authentication messages, host hash changes,
explicit links, and completion rollback. Storage-failure stopping is tested
while a second preparation and a database write lock are held. Launcher smoke
tests use synthetic values and ordinary test utilities without a real manager.

The [copied Docker evidence review](acceptance-review.md) confirms A01–A10,
local recovery, and the three legacy host tests. A10's saved output matches its
record and export hashes. The earlier export gap is closed by the later run.
Docker is absent from the devbox; review used the copied bundle.

HTTP/UI/API, scheduler execution and occurrence checkpoints, live reload,
runtime selection UI, age-based retention,
work-directory removal, restore tooling, custom environment/secrets, telemetry,
production migration, and deployment remain outside this implementation.


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

`ctl` reads one JSON request from standard input and prints one JSON response.
The Linux abstract Unix socket is scoped to the service name and effective UID.
Both ends check peer credentials. Use the same execution UID and network
namespace for the service and client. Names use 1–64 ASCII letters, digits,
hyphens, or underscores. Requests and responses are limited to 128 KiB.
The service allows 250 ms per socket read or write phase.

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
`service_status`.

SIGTERM, SIGINT, and the `shutdown` command stop new admission and dispatch.
The service continues polling active runs to durable completion. A storage
failure activates the shared safety gate and emergency stopping. An abrupt
process exit uses the existing recovery and review protocol on restart.

The Docker host runner includes a daemon case that submits through `ctl`,
checks matching retries and log reclamation, kills and restarts the daemon,
checks review and protected output, and checks planned shutdown with active
and queued work. It writes `evidence/daemon.json` after its assertions pass.
