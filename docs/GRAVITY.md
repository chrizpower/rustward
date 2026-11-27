# Rustward constraints and principles

Scope covers product direction and accepted architecture rules for Rustward.
The original decisions came from the user interviews on 2026-11-07 and
2026-11-08. Follow the [repository instructions](../AGENTS.md).

## Intent and use

Build a small task executor for one trusted administrator. Support generic
commands, manual requests, UTC cron schedules, and a web UI. Keep task and
schedule definitions in Git-managed TOML. Skip missed schedules and report gaps.
The target web UI accepts declared inputs, shows live output and retained
history, and supports operational controls. It leaves definition editing in
Git. Product requirements call for alerts on failure, timeout, interruption,
and schedule gaps. Keep workload credentials out of definitions, stored
inputs, and diagnostic data.

The main drivers are safe recovery, bounded resource use, fixed execution
inputs, and a small operational boundary. The administrator operates the service
and reviews uncertain work. Task owners remain responsible for external effects.

Follow accepted rules within scope. Proposals do not govern implementation. Cite
rule IDs in design reviews. Preserve IDs from the requirements reference. The decisions
below replace conflicting proposals in that plan, particularly planned shutdown
in P02. Other plan proposals remain proposals. Scenario checks below describe
required evidence; they are not test results.

## Domain language

Read [GLOSSARY.md](GLOSSARY.md) before work that uses domain concepts. Use
accepted terms within their contexts and follow the [domain language
maintenance rules](../AGENTS.md#domain-language-maintenance).

## Rules

### C01: Rust and generic execution

- **Type and status:** Constraint; confirmed product requirement and repository instructions.
- **Rule:** Implement Rustward in Rust. Use the same execution core for ordinary
  commands and Ansible. Keep Ansible-specific behavior outside the core. Keep
  production deployment in the deployment repository. Implementation does not authorize
  live tasks, migration, or deployment. Use synthetic inputs and credentials.
- **Reason and cost:** Preserve a reusable execution model. Workloads need their
  own definitions and runtime preparation.
- **Check:** Run a generic command and a synthetic Ansible task through the same
  path. Review dependencies for workload-specific coupling.
- **Revisit:** A product scope change requires an explicit decision.

### P01: One service with established Linux supervision

- **Type and status:** Principle; accepted in this interview.
- **Rule:** Use one Rust service, local SQLite, and embedded web assets. Require
  Linux with systemd for task supervision. Provide private proxy access for one
  administrator. The proxy authenticates access; Rustward protects state-changing
  browser requests. Target Debian 13 with systemd 257 and a dedicated unprivileged user manager,
  as selected by the user. Verify unit permissions on an isolated host before
  execution-core integration. Do not grant broad privileged unit creation as a shortcut.
- **Reason and cost:** Reduce custom supervision and deployment components.
  Availability depends on one host. Real execution tests need an isolated systemd
  host; a rootless development container cannot prove host integration.
- **Check:** Verify unit permissions, process cleanup, and concurrent run
  isolation. Verify proxy access enforcement and browser request protection.
- **Revisit:** A requirement for another platform, multiple administrators, or
  host failure tolerance changes this boundary.

Linux execution targets include x86-64 and ARM64. On macOS, build the shared
Rust code and run its unit tests. Keep systemd and Linux-specific interfaces
behind the Linux execution boundary. This scope follows the user's platform
decision on 2026-11-20.

### P02: Preserve accepted work and stop on uncertainty

- **Type and status:** Accepted principle.
- **Rule:** If external work is uncertain, preserve queued runs and block new
  starts until the administrator reviews and acknowledges it. Do not
  automatically retry interrupted runs. Local process cleanup is not proof
  that external work stopped. For planned restart or upgrade, pause new starts
  and wait for active runs to finish before restart. Unexpected shutdown uses
  recovery and review. A forced maintenance stop must not silently bypass this
  policy. Each acknowledgement records one completed run, its reviewed evidence,
  remaining uncertainty, and reason to resume. The administrator can explicitly accept
  remaining risk; acknowledgement does not prove external completion or change
  a run's result. Other dispatch blocks still apply.
- **Reason and cost:** Avoid unsafe overlap and repeated external changes.
  Review can delay the queue; draining can delay maintenance. Continuing after
  local cleanup would reduce delay but would not resolve external uncertainty.
- **Check:** Inject failure around task launch and completion. Confirm queued
  work remains blocked, interrupted work is not retried, and planned maintenance
  waits for active work. Test stale evidence and atomic acknowledgement.
- **Revisit:** Independent task survival across upgrades requires a separate
  lifecycle and output-capture decision.

### P03: Prepare fixed runtimes outside execution

- **Type and status:** Principle; accepted in this interview.
- **Rule:** An external step prepares fixed code and dependency revisions.
  Accepted requests retain their selected runtime. Updates must not change code
  or dependencies for queued or active work. Do not fetch code or install task
  dependencies as part of task execution.
- **Reason and cost:** Keep execution inputs stable and remove preparation
  failures from task startup. Updates need a separate preparation step and
  retained runtimes need storage management.
- **Check:** Update the selected runtime while work is queued and active.
  Confirm both retain their selection and dispatch fails if it cannot verify
  the selected runtime.
- **Revisit:** Automatic preparation or mutable code requires a new decision.

### P06: Stop on state or log storage failure

- **Type and status:** Principle; accepted in this interview.
- **Rule:** If state or log storage fails, close admission and dispatch and stop
  active local work. Require review when external work is uncertain. Do not
  report a durable success without evidence. Reaching a configured log size
  limit is normal truncation, not a storage failure; continue draining output.
- **Reason and cost:** Prevent unrecorded execution and continued loss of
  evidence. A storage fault can interrupt useful work. Letting active work
  finish would reduce interruption but can lose its result.
- **Check:** Inject metadata and log write failures. Confirm admission and
  dispatch close and local stopping does not depend on another successful
  database write. Check output flooding separately from storage faults.
- **Revisit:** A different active-work policy requires an explicit decision.

### P07: Enforce configurable resource bounds

- **Type and status:** Principle; defaults accepted in this interview.
- **Rule:** Start with one active run, a queue of 100 runs, a 10 MiB log
  limit per run, a 1 GiB aggregate retained-log limit, and 30-day retention.
  Retention cleanup must preserve queued and active run records and records
  needed for unresolved external review. Completed history can expire. Protect
  retained review output until acknowledgement. Reserve the full per-run log
  allowance before dispatch. Reclaim eligible completed logs in completion
  order to make room. Keep the run queued if protected output and existing
  reservations leave too little capacity.
  Make these limits configurable and enforce them. Metadata and transient
  buffers also need bounds; their values remain open. Exception accepted by the
  user on 2026-11-08: compact records for admitted manual request keys remain
  indefinitely with no fixed storage ceiling. Monitor their cumulative growth.
  Other metadata bounds and P06 storage-failure behavior still apply.
- **Reason and cost:** Prevent task activity from exhausting service resources.
  Bounds can reject requests or remove retained output. Protected logs can block
  dispatch. Permanent key records grow with admissions and require monitoring.
- **Check:** Exercise queue saturation, output flooding, and retention under
  load. Measure rather than infer memory and response-time behavior.
- **Revisit:** Change defaults using workload evidence. The plan's 64 MiB idle
  RSS and latency figures remain proposed measurement targets.

## Accepted console design rules

The [console design rules](DESIGN.md) are binding. When adding or changing a
console feature, follow the applicable rules for interaction, visual style,
accessibility, and review. Check the affected views in light and dark modes
before completing the change.

## Accepted configuration ownership and identity

The user accepted these rules on 2026-11-08.

Use separate service configuration, one TOML file per task, and one TOML file per
schedule. TOML is the sole source of definitions; SQLite retains runtime state.
Allow nested folders in both definition roots. Derive IDs from relative paths
without `.toml`, with separate task and schedule scopes. Each component starts
with a lowercase ASCII letter and then uses lowercase ASCII letters, digits, or
hyphens. Do not allow symlinks or a second ID field in definition files.

Validate the complete set before activation. Reject invalid updates as a whole
and retain the previous set. Preserve accepted runs across definition changes.
A move within a root changes identity; moving the root does not. A renamed
schedule gets no automatic transfer of pause state or occurrence history and
must not replay missed occurrences.

Reason and cost: Separate service settings from work definitions and allow
folder grouping without duplicate names for identity. Folder reorganization
changes IDs and requires reference updates. Review file and folder moves as
identity changes, not cosmetic edits. Check nested equal basenames, invalid
references, symlinks, rejected updates, and preservation of admitted work.
A change to definition ownership or identity requires a new user decision.

## Accepted telemetry scope

On 2026-11-08, the user accepted structured service logs and private
Prometheus-compatible `/metrics` for monitoring. Keep task output and sensitive
data out of service logs. Use bounded metric labels and keep the endpoint
private and read-only. P06 governs state and task-log failures. Production
collection belongs in the deployment repository.

## Accepted operational controls

The user accepted these effects in the glossary interview on 2026-11-08:

- Schedule pause stops new scheduled requests and records known occurrences as
  skipped. Manual requests and queued runs remain eligible. Resume does not replay
  skipped occurrences. A known pause alone is not a schedule gap.
- Dispatch pause stops new starts without stopping active runs or closing admission.
- Admission closed rejects requests from all triggers. By itself, it does not stop
  queued runs from dispatching or stop active runs.

The controls can be combined. Other safety blocks still apply.
See [the glossary](GLOSSARY.md).

## Admission and scheduling

Bind a manual request key at successful admission. Compare retries with the
original task ID and submitted inputs before defaults. A matching retry
returns the original run ID, including after history removal. Report removed
history explicitly. Reject changes to an admitted payload. A rejected request
can be corrected and evaluated again with the same key.

Use five-field UTC cron with numbers, lists, ascending ranges, and standalone
`*`. Reject steps and extensions. Require one day field to be `*`. Use an
established parser and evaluator. Each occurrence belongs to one schedule ID
and UTC due time. Track the schedule revision separately. Schedule edits
do not reopen handled occurrences. Skip missed work and report periods without
reliable evaluation as gaps. Keep at most one queued run per schedule within
the shared queue limit. Record further due occurrences as skipped while it
remains queued.

## Application-state restore

Restore begins with admission and dispatch closed. Review restored queued and
active work before any can execute. It may have completed after the backup.
Never replay restored work automatically. Start a new manual-key issuer
generation. Known old keys identify restored admissions. Reject unknown keys
from old generations. Normal restart preserves the issuer generation.

## Scenario checks

| Source and trigger | Component and conditions | Required response and success measure |
| --- | --- | --- |
| P02: Service dies during external work | Recovery; a run is active and other requests are queued | Preserve queued requests; start none before acknowledgement; retry no interrupted task automatically |
| P02: Operator requests planned maintenance | Dispatch; a task is active | Pause new starts; finish active work before restart |
| P03: Prepared runtime changes | Queue and executor; old requests remain accepted | Old requests retain the original runtime; new requests can select the new runtime |
| P06: A write fails | State or log storage; a task is active | Close admission and dispatch; stop local work; require review if external work is uncertain |
| P07: Output exceeds 10 MiB | Log capture; default limits apply | Mark truncation, continue draining, and keep buffers and retained output bounded |
| P02: Administrator accepts remaining risk | Review; an interrupted run has incomplete external evidence and dispatch is paused | Record evidence, uncertainty, affected runs, and reason; retain interrupted result; dispatch pause still prevents starts |
| P07: Retention age expires | History cleanup; an interrupted run needs external review | Preserve the review record; do not treat age alone as permission to delete it |
| Accepted controls: Schedule is paused | Scheduler; a due occurrence is known and evaluation remains reliable | Record a skipped occurrence, not a gap; do not replay it on resume |
| Admission contract: Old request is retried | Admission; run history is deleted but its permanent key record remains | Return original run ID and history-removed response; create no new run |
| Restore contract: Backup lacks a later admission | Restored service; old-generation key is unknown | Reject the key; create no run; keep restored queued work blocked for review |
| Retention contract: Protected logs fill allowance | Dispatch; a queued run cannot reserve its full log limit | Keep the run queued, report the block, and preserve protected output |
| Confirmed scope: Service misses cron times | Scheduler restarts after downtime | Execute no missed occurrence; report the gap and select a future occurrence |

## Release and production boundaries

The project uses the [Apache License, Version 2.0](../LICENSE). Workload compatibility, production access,
and migration require separate evidence and authorization. Distributed execution
and other scope extensions are not authorized.

Residual risks are uncertain external effects, host-specific unit permissions,
single-host availability, cumulative permanent-key storage, and faults that
prevent durable records. Resolve mechanisms with specifications and synthetic
failure tests. Scenario reasoning does not prove execution behavior.

## Changes and exceptions

Follow accepted rules within scope. For conflicts, use an already-authorized
specific revision or exception; otherwise propose a compliant path or ask for
the required decision. Continue unaffected work. A principle exception does not
waive a constraint or invariant.

Preserve rule IDs. Record the reason, scope, and authority for revisions and
retirements. Do not weaken rules to match existing code. Mark agent-derived
additions as proposals until accepted. Preserve temporary choices and their
known review conditions.

Later editors must use these selected practices, with or without the Gravity
skill. This is not a claim of full framework compliance:

- **QAW (Quality Attribute Workshop):** Check dominant drivers with scenarios.
  Record source, trigger, component, conditions, response, and success measure.
  Preserve known targets and mark unknown values.
- **TOGAF architecture principles:** State each rule, reason, permitted choices,
  and practical costs. Compare a plausible alternative for disputed rules.
- **ATAM (Architecture Tradeoff Analysis Method):** Check priority scenarios for
  sensitive decisions, quality interactions, risks, and missing evidence.
  Separate proposed targets, accepted requirements, and measured results.

Ask about unresolved choices that can change affected rules. Reuse known answers
and respect explicit deferrals. Use ASD-STE100 Simplified Technical English
adapted to software terminology.

2026-11-07: Accept C01, P01, P02, P03, P06, and P07 through the user interview.
P02 replaces the original stop-on-planned-shutdown proposal with draining. Add
standalone glossary maintenance guidance at the user's request.

2026-11-08: Accept the glossary and execution contracts.
Clarify P02 risk acknowledgement and accept separate operational controls,
restore review, lifecycle, scheduling, runtime, and retention behavior. P07
gains an explicit exception for permanent admitted manual-key records without a
fixed storage ceiling, replacing the earlier expiry decision. Other bounds and
P06 remain in force. No implementation or deployment is implied.

2026-11-08: Consolidate document roles and superseded-plan references at the
user's request. Preserve rule IDs, accepted decisions, and unresolved
mechanisms.

2026-11-08: Replace Rustward YAML definitions with TOML at the user's request.
Keep version 1 task fields and domain rules. This supersedes the earlier file
format choice and changes the Rust submitted-value type to `toml::Value`.
Preserve persisted identities and normalized snapshots. External workload
formats, including Ansible inventory, remain owned by those workloads.

2026-11-08: Select a dedicated systemd 257 user manager on Debian 13, with an
adapter for synthetic tests. Isolated-host execution evidence stays
required; transient-unit settings cannot weaken P02, P06, or P07.

2026-11-08: Accept separate service, task, and schedule files, TOML-only
definition ownership, nested folders, and relative-path IDs without duplicate
ID fields. Accept lowercase letter/digit/hyphen components, no symlinks, and
whole-set validation before activation. Preserve accepted work; a rename changes
identity. The later schema decision below supplies compatibility and loading rules.

2026-11-08: Accept normalized configuration revisions that exclude service
settings. Preserve stored revisions and request identity across schema changes.
Accept the five-field UTC cron subset in [admission and scheduling](#admission-and-scheduling).

2026-11-08: Accept FIFO, `NotQueued` cancellation races, and conservative recovery.
Require review and cleanup evidence.

2026-11-08: Start the execution timeout at dispatch. Use separate cleanup
grace. The first durable result wins, except that incomplete output changes
Success to Interruption and requires review.

2026-11-09: Accept one note for one completed run, with unchanged reviewed evidence.
See [review](ARCHITECTURE.md#administrator-review-and-acknowledgement).

2026-11-09: Bind service, manager, runtime policy, and private roots to one
non-root UID, with no new privileges or capabilities. Pin host dependencies and
use a clean environment. Preserve history.
