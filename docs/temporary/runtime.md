# Prepared runtimes

External preparation publishes fixed code and dependencies. Admission pins a
runtime ID. Dispatch verifies that selection and uses its admitted absolute
paths. See [Gravity P03](../GRAVITY.md#p03-prepare-fixed-runtimes-outside-execution).
The user accepted content-addressed manifests, separate preparation ownership,
and preserved task paths on 2026-11-09.

## Manifest, identity, and limits

A catalog contains `<runtime-id>/runtime.toml` for each externally prepared
runtime. The manifest has version 1, an optional `host` digest, a `directories`
list, and a `files` table. The controller requires host. The low-level verifier
keeps old manifests valid. See [host manifests](#host-manifest).

```toml
version = 1
directories = ["bin"]

[files."bin/task"]
sha256 = "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881"
executable = true
```

This illustrates the schema. The hash is for the single byte `x`, not a usable
program. Preparation must supply actual file hashes and executable content.
Every non-root folder, including empty folders, must be listed. Each file has a
lowercase 64-character SHA-256 and an optional executable boolean, default false.
The manifest itself is excluded from the file list to avoid a hash cycle. Files
must be nonempty as a set. Zero-byte files are permitted. Unknown fields,
duplicate keys/folders, invalid hashes, file/folder collisions, missing parent
folders, and entries named runtime.toml at the root are rejected.

Entry paths use normal relative UTF-8 components with `/`. Empty components,
absolute paths, dot components, backslashes, and NUL are rejected. Runtime paths
are not task IDs. Package filenames can contain uppercase letters, underscores,
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
caller can supply other finite limits. This does not change service TOML.

Existing stored runtime IDs, task snapshots, and fingerprints are never rewritten.
An opaque legacy ID that is not a manifest digest fails verification explicitly.
A missing or changed runtime never resolves to a newer runtime. Removing a
referenced runtime violates retention. Detection before launch records Failure.

## Ownership and preparation trust

`Catalog::open(path, Protection, Limits)` requires an absolute normal catalog path.
Protection names preparation and execution UIDs. They must differ, and execution
UID 0 is rejected. Runtime files and folders belong to the preparation UID. All
ancestors belong to that UID or root and cannot be writable by other users.
Kernel `openat2` rejects symlinks in every path component. Catalog roots are
checked again during verification. Held descriptors are compared with current
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
that condition. A nonzero UID alone is insufficient.

The trusted preparation user can still change owned files. It must never modify
or replace published runtimes and must retain selections referenced by queued
or active runs. Verification is not an atomic filesystem snapshot or a defense
against that trusted user. No runtime deletion or preparation tool is added.

Verification is synchronous filesystem I/O and has no fixed latency guarantee.
The controller performs it in a preparation worker, separate from emergency
stopping.

The preparer must supply a complete fixed dependency selection. Manifest checks
cannot discover arbitrary interpreters, dynamic libraries, imports, subprocesses,
or network dependencies. The [controller](service.md#execution-controller) adds pinned host manifests and a controlled
environment.

## Task paths and writable directories

Both executable and working_directory keep their admitted absolute paths. The
executable must be a listed executable file inside the selected runtime. The
working directory must be that runtime root or a listed folder. No path is
remapped, and no argument is changed. Selecting another runtime path requires a
new task definition. Already admitted work keeps its original snapshot and path.

`WorkRoot::open(path)` takes a separate private directory owned by the service
user with mode 0700. Ancestors must belong to the service user or root. Writable
ancestors require the sticky bit. Kernel path resolution rejects symlinks.
`create(&run_id)` exclusively creates a 0700 folder, synchronizes it and its
parent, and returns WorkDirectory. Existing files, folders, and symlinks are
never reused. `check` rejects changed ownership, permissions, or directory
identity. Root and child replacement are detected. Setup failure can leave
partial evidence. Dropping a handle never deletes data.

A run's writable directory is outside the catalog and does not replace its
working_directory. These directories separate run data. They are not a security
boundary between hostile tasks running as the same user. Removal and retention
remain future work. Work-storage errors set the shared failure gate when consumed
by dispatch. Invalid or reused paths record a definite pre-launch failure.

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
user or set environment variables. The [controller](service.md#execution-controller) performs
verification in workers and binds actual user identity, private roots, host
selection, manager, and environment. Secret wiring remains unimplemented.

### Pre-launch failure handling

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

The caller supplies a code from `dispatch::LaunchFailure`. This method does not
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
remain valid. Old records are not rewritten. A version 5 SQL fixture checks
migration, historical runtime IDs, fingerprints, sequence order, and exact
acknowledgement retries.

Focused tests cover every failure code, no supervisor calls, capacity retention,
queued admission identity, rollback after failed or ignored writes, conservative
recovery, an earlier cancellation, and dispatch-owner mismatch.

## Host manifest

Runtime version 1 manifests can include `host = "<host-id>"`. It participates in
the runtime hash. When absent, serialization omits it, so existing runtime IDs
stay unchanged. The low-level runtime verifier still accepts old manifests.
The controller requires a host reference. It never chooses another runtime or
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
runs use them. An operating-system update must wait for those runs. A queued run
whose host selection no longer matches fails before launch. Verification is not
an atomic snapshot against a trusted preparer that changes files concurrently.
No preparation, freeze enforcement, deletion, or deployment tool is supplied.

## Verification and open work

Synthetic tests cover manifest normalization, protected paths, command paths,
selection changes between admissions, and revalidation before launch. The
[suite](acceptance.md) runs the opt-in runtime permission test and A08 with
pinned A and B selections. Runtime publication, dependency completeness, and
preserving referenced versions remain preparation responsibilities. Safe
removal of writable run directories and prepared-runtime retirement need
operational procedures.
