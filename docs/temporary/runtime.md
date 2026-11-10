# Prepared runtime verification work

Status: Implemented Rust library. The user accepted content-addressed,
separately owned runtime directories and preserved task paths on 2026-11-09.
Tests use synthetic files and fake supervision. No real task execution or Ansible
acceptance case is authorized or proved by these tests.

Transfer stable runtime interfaces to ARCHITECTURE.md, policy decisions to
GRAVITY.md, and agreed domain meanings to GLOSSARY.md before removal. Resolve
or transfer open decisions and preserve acceptance evidence and inbound links.

## Accepted requirements

Prepare fixed code and dependencies outside execution. Retain the selected
runtime for queued and active runs. Never fetch dependencies during execution or
substitute another runtime when the selected runtime is missing. Give each run
a separate writable directory outside its runtime. Runtime preparation,
production deployment, and migration of workloads remain separate work.

The Ansible acceptance specification is the early integration milestone.
Runtime verification must precede connecting real execution to its local
playbook and ordinary-command cases. The [local runner](acceptance-runner.md)
and A01/A02 fixtures are implemented; no real-systemd acceptance case has passed.

## Implemented pre-launch failure handling

`Session::fail_before_start(permit, LaunchFailure)` consumes an unstarted permit
from that session. It records CleaningUp/Failure, a rejected start state,
confirmed cleanup because no process was started, and one fixed diagnostic code:

- runtime_missing
- runtime_invalid
- runtime_changed
- runtime_unsafe
- runtime_command
- work_directory
- work_storage

The caller supplies a code from `dispatch::LaunchFailure`; this method does not
inspect runtime files or create work directories. It cannot replace an earlier
result or use another session's permit. It never calls the supervisor or returns
a permit for retry. A work-storage failure also sets the shared Safety gate.

Capacity remains reserved until output handling finishes and `finish` commits
completion. Existing admitted request identities and matching retries remain
unchanged. A failed metadata write rolls back the decision, closes dispatch,
and preserves recovery uncertainty. Local no-start knowledge prevents an
emergency poll from attempting to stop an execution that this permit never
started. That in-memory knowledge is not a durable cleanup record after restart.

Schema version 6 adds nullable launch_failure evidence. New review evidence
includes this field when present. Encoding 1 acknowledgement records without it
remain valid; old records are not rewritten. A version 5 SQL fixture checks
migration, historical runtime IDs, fingerprints, sequence order, and exact
acknowledgement retries.

Focused tests cover every failure code, no supervisor calls, capacity retention,
queued admission identity, rollback after failed or ignored writes, conservative
recovery, an earlier cancellation, and dispatch-owner mismatch.

## Manifest, identity, and limits

A catalog contains `<runtime-id>/runtime.toml` for each externally prepared
runtime. The manifest has version 1, an optional `host` digest, a `directories`
list, and a `files` table. The controller requires host; the low-level verifier
keeps old manifests valid. See [host manifests](controller.md#runtime-and-host-dependencies).

```toml
version = 1
directories = ["bin"]

[files."bin/task"]
sha256 = "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881"
executable = true
```

This illustrates the schema; the hash is for the single byte `x`, not a usable
program. Preparation must supply actual file hashes and executable content.
Every non-root folder, including empty folders, must be listed. Each file has a
lowercase 64-character SHA-256 and an optional executable boolean, default false.
The manifest itself is excluded from the file list to avoid a hash cycle. Files
must be nonempty as a set; zero-byte files are permitted. Unknown fields,
duplicate keys/folders, invalid hashes, file/folder collisions, missing parent
folders, and entries named runtime.toml at the root are rejected.

Entry paths use normal relative UTF-8 components with `/`. Empty components,
absolute paths, dot components, backslashes, and NUL are rejected. Runtime paths
are not task IDs; package filenames can contain uppercase letters, underscores,
and dots. No symlinks, sockets, devices, or FIFOs are accepted.

`runtime::manifest_id(source)` applies defaults, sorts directories and file keys,
and serializes sorted-key JSON. Hash the exact bytes `rustward-runtime-v1`
followed by that JSON with SHA-256. The lowercase digest is the runtime ID.
Comments, formatting, and declaration order do not change it. File content,
executable flags, paths, folders, and host when present participate. An absent
host is omitted from normalized JSON, preserving existing runtime IDs. There is no ID field in the
manifest. Publish the manifest and tree under that ID after preparation finishes.

`runtime::Limits` bounds parsing and traversal. Defaults are 16 MiB manifest
bytes, 100,000 entries including the manifest, 4 GiB total file content, 64 path
components, and 4096 bytes per relative path. Hashing uses a 64 KiB buffer. The
caller can supply other finite limits; this does not change service TOML.

Existing stored runtime IDs, task snapshots, and fingerprints are never rewritten.
An opaque legacy ID that is not a manifest digest fails verification explicitly.
A missing or changed runtime never resolves to a newer runtime. Removing a
referenced runtime violates retention; detection before launch records Failure.

## Ownership and preparation trust

`Catalog::open(path, Protection, Limits)` requires an absolute normal catalog path.
Protection names preparation and execution UIDs. They must differ, and execution
UID 0 is rejected. Runtime files and folders belong to the preparation UID; all
ancestors belong to that UID or root and cannot be writable by other users.
Kernel `openat2` rejects symlinks in every path component. Catalog roots are
checked again during verification; held descriptors are compared with current
paths to detect observed replacement.

Group/other write bits, set-ID bits, sticky bits inside a runtime, and file
capability attributes are rejected. Files and directories must provide the
required group/other read and execute access. A non-executable manifest entry
cannot have executable mode bits. These rules also reject effective ACL write
access through the [Linux ACL permission mask](https://man7.org/linux/man-pages/man5/acl.5.html).
Traversal checks every entry, rejects
unlisted content, streams file hashes, and checks observed metadata changes.
The manifest is checked again after traversal.

The actual execution user must have no [capabilities](https://man7.org/linux/man-pages/man7/capabilities.7.html)
that bypass filesystem ownership or access checks. Host verification must establish
that condition; a nonzero UID alone is insufficient.

The trusted preparation user can still change owned files. It must never modify
or replace published runtimes and must retain selections referenced by queued
or active runs. Verification is not an atomic filesystem snapshot or a defense
against that trusted user. No runtime deletion or preparation tool is added.

Verification is synchronous filesystem I/O and has no fixed latency guarantee.
The controller performs it in a preparation worker, separate from emergency
stopping.

The preparer must supply a complete fixed dependency selection. Manifest checks
cannot discover arbitrary interpreters, dynamic libraries, imports, subprocesses,
or network dependencies. The [controller](controller.md) adds pinned host manifests and a controlled
environment. Isolated-host evidence remains required. A directory hash alone does not prove
that an arbitrary program uses only declared files.

## Task paths and writable directories

Both executable and working_directory keep their admitted absolute paths. The
executable must be a listed executable file inside the selected runtime. The
working directory must be that runtime root or a listed folder. No path is
remapped, and no argument is changed. Selecting another runtime path requires a
new task definition; already admitted work keeps its original snapshot and path.

`WorkRoot::open(path)` takes a separate private directory owned by the service
user with mode 0700. Ancestors must belong to the service user or root; writable
ancestors require the sticky bit. Kernel path resolution rejects symlinks.
`create(&run_id)` exclusively creates a 0700 folder, synchronizes it and its
parent, and returns WorkDirectory. Existing files, folders, and symlinks are
never reused. `check` rejects changed ownership, permissions, or directory
identity. Root and child replacement are detected. Setup failure can leave
partial evidence; dropping a handle never deletes data.

A run's writable directory is outside the catalog and does not replace its
working_directory. These directories separate run data; they are not a security
boundary between hostile tasks running as the same user. Removal and retention
remain future work. Work-storage errors set the shared failure gate when consumed
by dispatch; invalid or reused paths record a definite pre-launch failure.

## Dispatch interface

`Session::prepare_runtime(permit, &catalog, &work_root)` consumes a one-use permit,
verifies its retained runtime and command, and creates its writable directory.
It returns PreparedPermit with unchanged launch data, runtime ID, expected
execution UID, and work-directory path. Preparation cannot use another session's
permit or replace a prior cancellation. Pause, review, timeout, and storage gates
retain their existing meanings.

After capture setup, `start_prepared(prepared, supervisor, output)` verifies the
runtime and work-directory identity again, then uses the existing one-call start
protocol. A failed check records Failure and no-start cleanup with a launch
failure code. A failed metadata write reports State error rather than claiming
the runtime failure was saved. No SQLite transaction spans file verification or
a supervisor call. Output handling must still finish before slot release.

PrepareError distinguishes runtime checks from state/storage errors. Prepared
permits cannot be cloned or recovered. Existing low-level `start` remains the
protocol interface for tests. These low-level APIs do not select a systemd
user or set environment variables. The [controller](controller.md) performs
verification in workers and binds actual user identity, private roots, host
selection, manager, and environment. Secret wiring remains unimplemented.

## Evidence and remaining work

Tests cover complete trees, normalization, modes, ownership policy, capability
checks on ordinary files, symlinks and ancestors, special files, invalid paths,
resource bounds, root relocation, command escapes, separate work directories,
reuse/replacement, retained runtime A after selecting B, missing-runtime failure,
revalidation before start, preserved retries, and failed failure-record writes.
Numeric UID fixtures model another execution user without changing process UID.
No configured task is executed.

The opt-in runtime permission test requires a protected, separately prepared
synthetic fixture and a private service-owned work root on an isolated host:

```sh
cargo test --locked --test runtime_host -- --ignored --exact prepared_runtime_host_permissions
```

Supply RUSTWARD_PREPARATION_UID, RUSTWARD_RUNTIME_CATALOG, RUSTWARD_RUNTIME_ID,
RUSTWARD_RUNTIME_PROGRAM, RUSTWARD_RUNTIME_CWD, and RUSTWARD_RUNTIME_WORK_ROOT.
Run it as the non-root execution user. It checks native write denial and writable
run data without executing a program. It has not been run here. This test does
not replace systemd cleanup/capture tests or acceptance cases A01–A10.

The Rust APIs take catalog/work paths and UID policy explicitly. Service TOML,
activation, and runtime selection UI are unchanged. The controller now connects
UID binding, host dependencies, capture, and controlled environment. Real
execution acceptance, secret handling, systemd host verification, retention
cleanup, scheduler execution, and reload remain unimplemented or unverified.
The A01/A02 harness exists but has not passed real-systemd acceptance. Production migration and deployment are outside
this slice.
