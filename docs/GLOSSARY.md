# Rustward glossary

Shared terms for Rustward requirements, code, tests, and operator discussions.
These meanings were agreed in the user interviews of 2026-11-07 and 2026-11-08
and the implemented configuration contract. Unconfirmed meanings must be marked
proposed. Separate bounded contexts have not been assigned.

A definition does not prove implementation. See [architecture](ARCHITECTURE.md)
for implementation status and [Gravity](GRAVITY.md) for accepted policy. Follow
[domain language maintenance](../AGENTS.md#domain-language-maintenance) when
changing terms. Keep mechanisms, code mappings, and work lists outside this glossary.

### Tasks and inputs

**Rustward**: The generic task executor being built in this repository.

**Administrator**: The one trusted person who operates Rustward and reviews
uncertain work. Operator names the same role in existing documents.

**Task** or **task definition**: A reusable definition of work with an ID,
label, executable, working directory, arguments, and input schema.

**Task ID**: The identifier used to select a task definition, unique within the
complete task set. Under the accepted file layout, it is the path relative to
the task root without `.toml`. It remains stable while that relative path stays
the same; moving or renaming the file within the root changes its identity.
The version 2 loader implements this identity rule.

**Label**: Human-readable text for a task. It does not replace the task ID.

**Configuration**: The service settings and complete set of task and schedule
definitions. The version 2 loader reads these from separate TOML files.

**Service configuration**: Settings for Rustward itself, including definition
locations, storage paths, global resource limits, and retention. It is separate
from task and schedule definitions.

These configuration meanings and path-based identities were accepted by the
user on 2026-11-08 to separate service settings and allow grouped definitions
without duplicate ID fields. The [file contract](temporary/configuration.md#accepted-file-layout)
owns syntax, validation, and migration work.

**Configuration validation**: Checking a configuration against its schema and
reference rules. It does not check program availability or execute a command.

**Input schema**: The declared inputs for a task, with their types, required
flags, defaults, and permitted choices.

**Input value**: A typed value supplied for a declared input, or its default
when no value is supplied. Inputs are data. TOML has no null value; an omitted value selects its
default when one exists. An optional input can be absent when no argument
references it.

**Required input**: An input that must have a value after defaults are applied.
It need not be supplied explicitly if it has a default.

**Choice list**: The permitted values for an input. A choice list constrains its
declared type; it is not a separate input type.

**Request input validation**: Checking supplied values against the selected
task's input schema, including inputs that no argument references.

**Executable**: The absolute path to the program selected by a task definition.

**Argument**: One value in the ordered list passed to a program. In version 1,
each configured entry produces exactly one argument. A string with spaces
remains one argument.

**Literal argument**: An argument whose value comes directly from the task
definition.

**Input reference**: An argument entry that selects a declared input by ID and
uses its complete value. Repeated references produce repeated arguments. A
referenced input must be required or have a default.

**Argument generation**: Producing the ordered argument values after input
validation. It performs no shell expansion, splitting, or interpolation.

**Working directory**: The absolute directory path configured for command
execution. Validation does not check whether it exists.

### Requests and runs

**Request**: A submission asking Rustward to create a run for a task with
specified inputs. A rejected request creates no run and receives no run ID.

**Admission**: The decision to accept a request and durably record its queued
run. Admission creates the run and assigns its run ID.

**Run ID**: The stable identity of a run, assigned at admission and retained
through queueing, execution, and completion.

**Run**: One accepted execution of a task, from queue entry through its final
result. A run can be cancelled before its program starts. Two distinct admitted
requests for the same task create two runs. A submission retry identifies the
existing run. Running describes a state, not all runs.

**Submission retry**: Sending the same request again with the same request key
and payload. If already admitted, it returns the existing run ID.

**Execution retry**: Explicitly requesting a new run to repeat work from an
earlier run. It receives a new run ID. Automatic retries of interrupted work
remain prohibited by P02.

**Queue**: Accepted runs waiting for dispatch.

**Dispatch**: The transition that reserves an execution slot and starts the
launch process. The run leaves the queue at this point.

**Execution slot**: One unit of permitted local execution capacity, held through
launch, execution, and local cleanup. It is released only when local cleanup is
confirmed. Uncertain external work can still block further dispatch after
release.

**Active**: A summary term covering launch, program execution, and local
cleanup. An active run holds an execution slot.

**Running**: The program has started and has not yet been observed to exit.

**Completed**: Rustward has recorded a final result and confirmed local cleanup.
Completed does not mean successful. A run cancelled before dispatch can complete
without becoming active.

**Interrupted**: A result indicating that Rustward cannot establish the normal
execution result, for example after a crash. An interrupted run can remain
active while cleanup is pending.

**Success**: The program reports a successful exit under the task's result
contract. It does not independently prove the intended external effect occurred.

**Failure**: Launch fails, or the program reports an unsuccessful exit.

**Cancellation**: An explicit stop request that prevents a queued run from
starting or initiates stopping active work. It does not prove remote work
stopped.

**Timeout**: The configured execution time limit is reached, which initiates
stopping active work. It does not prove remote work stopped. New tasks default
to one hour from dispatch. Admissions retain their selected time limits. The user
accepted this default on 2026-11-08; see [stopping](temporary/stopping.md).

Avoid **job** as a synonym within Rustward and **attempt** as a separate entity.
Preserve external interface names where required. Each execution retry is a new
run; a failed launch holds its slot until local cleanup is confirmed.

### Scheduling and controls

**Schedule**: A rule that selects a task, fixed inputs, and UTC times at which
to request runs.

**Schedule ID**: The identifier of a schedule, unique within the complete
schedule set. It is the path relative to the schedule root without `.toml`.
Moving or renaming a file within that root changes its identity. Task and
schedule IDs have separate scopes.

**Occurrence**: One due time of a schedule. It exists even if no request is
admitted.

**Trigger**: The origin of a request: manual submission or a schedule
occurrence.

**Schedule gap**: An interval during which Rustward did not evaluate a schedule
reliably. Its missed occurrence count can be unknown.

**Skipped occurrence**: A known occurrence for which no run is admitted, with a
reason. A known omission does not by itself establish a schedule gap.

**Schedule pause**: Stops new scheduled requests. Known due occurrences are
recorded as skipped. Manual requests and queued runs remain eligible. Resume
does not replay skipped occurrences. A known pause alone is not a schedule gap.

**Dispatch pause**: Prevents queued runs from starting. It does not stop active
runs or, by itself, reject new requests.

**Admission closed**: Rejects new requests from all triggers. It does not, by
itself, stop queued runs from dispatching or stop active runs.

**Planned maintenance**: An intended restart or upgrade. Accepted policy pauses
new starts and waits for active work to finish.

An outage can produce a gap with known skipped occurrences. Queue rejection can
produce a skipped occurrence without a gap. Pause controls are independent and
can be combined; other safety blocks still apply.

### External work and review

**Local cleanup**: Confirming that all local processes belonging to a run have
ended, including child processes. It does not remove logs, runtime files, or
remote resources, and does not prove external work stopped.

**External work**: Activity or effects outside the local processes supervised
for a run, such as a remote backup or service change.

**External outcome**: The actual effect of work outside Rustward's local
supervision, distinct from the local execution result.

**Uncertain external work**: Rustward lacks sufficient evidence to establish the
relevant external activity or outcome. Accepted P02 policy blocks new starts
until administrator review and acknowledgement. Task-specific evidence can
resolve uncertainty; acknowledgement can instead accept the remaining risk.
Local exit alone does not resolve it.

**Reconciliation**: Comparing recorded run state with available local and
external evidence, then recording what is known, what remains uncertain, and
what action is needed.

**Acknowledgement**: The administrator's recorded decision that dispatch may
resume for one run’s reviewed external uncertainty. Rustward records the run,
reviewed evidence, and time. One administrator note states the checks, remaining
uncertainty, and reason to resume. It can accept remaining risk; it does not assert remote completion or change an interrupted result to
success. Other dispatch blocks still apply. One-note records and local completion
before acknowledgement were accepted by the user on 2026-11-09; see
[the review contract](temporary/review.md).

Stopping a local SSH client does not prove a remote backup stopped. A
persistent, known file change is an external effect, but persistence alone is
not uncertainty.

### Runtimes and revisions

**Prepared runtime**: Fixed code and dependencies prepared outside task
execution. Accepted work retains its selected runtime.

**Runtime preparation**: The external step that prepares fixed code and
dependency revisions before execution.

**Runtime identity**: Identifies one fixed selection of prepared code and
dependencies. Changing that selection creates a different identity.

**Runtime retention**: Keeping prepared runtimes while accepted runs still
reference them.

**Task definition revision**: Identifies one version of a task definition's
normalized content, including its ID. A change to that content, including its
label, creates a different revision. Apply schema defaults before computing that content; an
omitted field and an explicit equivalent default have the same revision.

**Configuration revision**: Identifies one version of the complete set of normalized
task and schedule definitions, including their derived IDs. Service settings and
definition-root paths do not participate. A changed definition changes the revision.
This boundary was accepted in the configuration implementation discussion on
2026-11-08: moving a root preserves the revision; moving a definition changes it.
Stored earlier revisions retain their original meanings.

Comments, whitespace, and TOML key order do not change definition revisions.
Argument order does. Request comparison instead uses submitted values before
defaults; see [request identity](ARCHITECTURE.md#request-identity).

### Logs and retention

**Retention**: The policy that determines how long completed-run records and
output remain available.

**Per-run log limit**: The maximum output retained for one run. Reaching it
causes truncation; output must still be drained.

**Aggregate log limit**: The maximum total stored log data across runs.

**Log truncation**: Stopping retention of output at a configured limit while
continuing to drain output. This is normal limit handling.

**Retention cleanup**: Removing records or output eligible for deletion under
the retention policy. It is separate from local process cleanup. Queued and
active run records, and records needed for unresolved external review, are
protected. Retained output needed for unresolved review is protected until
acknowledgement. Completed history can expire.

**Storage failure**: A failure to store state or logs. Accepted policy closes
admission and dispatch and stops active local work.
