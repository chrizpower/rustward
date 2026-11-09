# Prepared runtime verification work

Status: In progress. Pre-launch failure handling and its migration are
implemented. Runtime verification and run work-directory creation are not yet
implemented. The manifest, identity, ownership, and path choices below remain
proposals awaiting user answers. P03 and the prepared-runtime contract in
execution.md remain accepted and authoritative.

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
playbook and ordinary-command cases. The harness and fixtures are not yet
implemented; no acceptance case has passed.

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

## Proposed manifest and identity rules

Use externally prepared runtime directories. A versioned runtime.toml manifest
lists the complete tree and SHA-256 hashes. Derive a runtime ID from its normalized
manifest. Verify the tree before launch; reject changed or unlisted content,
symlinks, special files, and paths writable by the execution user. A separate
trusted preparation user owns the runtime. Verification does not build runtimes.

Do not rewrite existing stored runtime IDs. An old ID that cannot identify a
verifiable runtime must fail explicitly; never resolve it to a current selection.
The new ID format and ownership rules are not accepted yet. Manifest syntax,
resource bounds, dependency completeness, and preparation-user trust must be
specified before implementation can claim these guarantees.

## Proposed task and work-directory rules

Preserve the task's absolute executable and working_directory paths. Require both
to be inside the selected runtime. Create a separate writable directory per run
and expose its path through the Rust API. Environment wiring belongs to the
execution controller. Selecting a runtime at another path requires a new task
definition; admitted runs retain their original paths.

This path behavior is not accepted yet. Do not silently remap task paths, change
the process working directory, or treat these proposals as implemented behavior.
The existing task path validation and argument generation remain unchanged.

## Remaining work

Resolve the user-visible choices, then implement bounded verification, protected
path handling, separate work directories, and retained-runtime tests. Verify
ownership using synthetic fixtures before isolated host evidence. Connect these
interfaces to capture and the controller only after their required gates pass.
Real execution, environment and secret references, host permission verification,
retention cleanup, scheduler execution, reload, and the Ansible acceptance harness
remain unimplemented. Production migration and deployment are outside this slice.
