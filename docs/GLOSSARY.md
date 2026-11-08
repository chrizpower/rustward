# Rustward glossary

Scope: Rustward application concepts. Bounded-context boundaries remain open.

Use this glossary as the shared language for requirements, code, tests, and
operator discussions. Domain experts and developers define this language
together. In domain-driven design, this is the ubiquitous language: within a
bounded context, the same concept has the same name across these uses.

Definitions come from the [configuration contract](temporary/configuration.md), [accepted
architecture rules](GRAVITY.md), and [confirmed scope](temporary/requirements.md#purpose-and-authority). A
definition does not mean that a feature is implemented. See the [architecture
overview](ARCHITECTURE.md) for implementation status. Contract status is listed
below.

## Terminology principles

The user selected the following guidance for Rustward. Source attributions in
this table are supplied by the user.

| Principle | Status | Source |
| --- | --- | --- |
| Use shared, precise domain language. | Foundational DDD | Eric Evans, 2003 |
| Reflect the domain language in code. | Foundational DDD | Eric Evans, 2003 |
| Scope language to bounded contexts. | Foundational DDD | Evans, Vernon |
| Evolve language as domain understanding improves. | Foundational DDD | Evans |
| Remove ambiguity and inconsistencies. | Explicit DDD guidance | Evans, Fowler |
| Use imperative names for commands and past tense for events. | Widely used convention | CQRS / EventStorming |
| Maintain a formal glossary. | Optional practice, selected for Rustward | Not required by DDD |
| Enforce terms with automated tooling. | User recommendation | Not a foundational DDD practice |

A bounded context is a boundary within which a domain model and its language
have a defined meaning. This document currently covers Rustward application
concepts; it does not establish separate bounded contexts. If contexts need
different meanings for a term, record each meaning and its context explicitly.
Resolve inconsistent meanings within a context instead of treating them as
synonyms.

Use agreed terms in code names, tests, documentation, and discussions. For a
domain command, use an imperative name such as `CancelRun`. For a domain event,
use a past-tense name such as `RunCancelled`. These names are examples, not
implemented interfaces or accepted state transitions. This convention does not
require CQRS or event sourcing.

Automated terminology checks are recommended but are not implemented by this
document. Select their scope after terms are agreed. Use review to check
meaning; a text match alone cannot establish consistent domain language.

## Language

These meanings come from the implemented configuration contract and the user
interviews of 2026-11-07 and 2026-11-08. All entries apply to Rustward
application concepts; this scope does not establish separate bounded contexts.
Future unconfirmed definitions must be marked proposed.

### Tasks and inputs

**Rustward**: The generic task executor being built in this repository.

**Administrator**: The one trusted person who operates Rustward and reviews
uncertain work. Operator names the same role in existing documents.

**Task**: A reusable definition of work, including the program, arguments, and
input schema. Task definition is the explicit form of this term.

**Task definition**: The explicit name for a task: its ID, label, executable,
working directory, arguments, and input schema describe reusable work.

**Task ID**: The stable identifier used to select a task definition. Version 1
requires it to be unique within a configuration.

**Label**: Human-readable text for a task. It does not replace the task ID.

**Configuration**: A versioned YAML document containing task definitions.
Version 1 contains no schedule definitions.

**Configuration validation**: Checking a configuration against its schema and
reference rules. It does not check program availability or execute a command.

**Input schema**: The declared inputs for a task, with their types, required
flags, defaults, and permitted choices.

**Input value**: A typed value supplied for a declared input, or its default
when no value is supplied. Inputs are data. Explicit null is invalid and does
not select a default. An optional input can be absent when no argument
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
remains one argument. The private code type `ArgumentSource` describes its configured source.

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
stopping active work. It does not prove remote work stopped.

Avoid **job** as a synonym within Rustward and **attempt** as a separate entity.
Preserve external interface names where required. Each execution retry is a new
run; a failed launch holds its slot until local cleanup is confirmed.

### Scheduling and controls

**Schedule**: A rule that selects a task, fixed inputs, and UTC times at which
to request runs.

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
resume for reviewed external uncertainty. It identifies affected runs, evidence,
remaining uncertainty, and the reason for resuming. It can accept remaining
risk; it does not assert remote completion or change an interrupted result to
success. Other dispatch blocks still apply.

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
parsed content. Changing its normalized content, including its label, creates a
different revision. Apply schema defaults before computing that content; an
omitted field and an explicit equivalent default have the same revision.

**Configuration revision**: Identifies one version of the complete set of parsed
definitions. A changed definition changes the configuration revision.

Comments, whitespace, and YAML key order do not change definition revisions.
Argument order does. See the [revision
contract](temporary/execution.md#definition-revision-contract) for the distinction
between normalized definitions and submitted request values.

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

## Current code mappings

These mappings describe current usage in [src/config.rs](../src/config.rs). Code
provides evidence of usage; it does not decide domain meaning.

| Code name | Current meaning | Remaining distinction |
| --- | --- | --- |
| `TaskDefinition` | A task definition decoded from configuration. | Task names reusable work; run names one accepted execution. |
| `InputDeclaration` | A declared input with type, required flag, default, and choices. | Distinguish the declaration from a supplied input value. |
| `ArgumentSource` | A configured literal or input reference. | Distinguish this entry from the generated string passed to a program. |
| `Config` | Validated task configuration. | `Document` represents the YAML document before validation. |

`Config::arguments` takes submitted values without changing them. The private
`TaskDefinition::validate_submitted_values` method checks all input declarations
and resolves typed input values from submitted values or defaults. Argument
generation then converts referenced values to strings. Unused input values are
validated but do not produce arguments. Generated arguments are `Vec<String>`;
they are not task definitions or configured argument sources.

The private names were aligned with the accepted meanings during the code
review requested on 2026-11-08. Public Rust interfaces and YAML version 1 fields
are unchanged. These mappings do not authorize public interface changes.

## Contract status

The terminology and main policy choices were agreed with the user on 2026-11-08.
[Execution contracts](temporary/execution.md#admission-contract-decisions)
records admission, lifecycle, scheduling, retention, restore, runtime, and
revision contracts. Keep mechanisms in that document rather than this glossary.

[Remaining specifications and
evidence](temporary/implementation.md#remaining-specifications-and-evidence) include
canonical encoding, atomic transitions, systemd integration, scheduling
checkpoints, runtime manifests, and backup consistency. These are implementation
gates, not evidence that behavior works. New policy trade-offs need review.

Bounded-context boundaries remain unassigned. Ask about a boundary when a
concrete difference in domain meaning requires it; do not infer one from
modules.

## Use and maintain

Use the precise term when a distinction matters, such as task definition versus
input value. Preserve required public interfaces when terminology changes.

After a terminology decision, update its definition and any proposed marker in
the language section and record its source. Keep implementation contracts and
accepted policy in their source documents. A glossary edit alone must not accept
a proposed behavior. Use examples to resolve ambiguity before introducing new
synonyms. Revise definitions as domain understanding improves. Check affected
code, tests, and documents for inconsistent usage when a definition changes.
Record context boundaries when they become necessary; do not infer them from
module names alone.

During every task that uses domain concepts:

- Read relevant entries, use accepted terms, and avoid listed alternatives within
  their contexts. Preserve
  external interface names and explain their mappings when needed.
- Ask the user about a new concept, ambiguous use, changed meaning, or obsolete
  term. Propose wording and explain its effect with a concrete example. Use a
  structured question tool when available.
- Reuse known answers and authorization. Wait for required answers before
  dependent work. Silence is not acceptance. Continue independent work and
  record deferred meanings and consequences without repeatedly asking.
- Check definitions and relationships with normal and boundary scenarios.
  Compare them with code and report differences. Identify when domain expertise
  is missing; code does not establish intended meaning.
- Record resolved changes during the task, with their source and reason. Keep
  unconfirmed meanings proposed. Check affected references and record remaining
  naming differences after changes or deletions. Do not rename unrelated code or
  change public interfaces without authorization. If edits are outside scope,
  present the proposed glossary change instead.

Determine context boundaries from domain meaning, not repository or deployment
structure. Include technical terms only when they have a project-specific
meaning. No task must complete the whole glossary. These practices do not
require aggregates, repositories, events, or other DDD patterns.

## DDD sources

References for the project terminology principles:

- [Eric Evans, Domain-Driven Design Reference](https://www.domainlanguage.com/ddd/)
- [Martin Fowler, Ubiquitous Language](https://martinfowler.com/bliki/UbiquitousLanguage.html)
- [Vaughn Vernon, Domain-Driven Design Distilled](https://www.informit.com/store/domain-driven-design-distilled-9780134434421)
