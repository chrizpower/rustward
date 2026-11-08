# Rustward constraints and principles

Scope: Application architecture for the first version of Rustward. Status:
Mixed. Accepted decisions and proposals are marked below. Decision source: User
interviews on 2026-11-07 and 2026-11-08. Parent guidance:
[AGENTS.md](../AGENTS.md). Design context: [Requirements](temporary/requirements.md).

## Intent and use

Build a small task executor for one trusted administrator. Support generic
commands, manual requests, UTC cron schedules, and a web UI. Keep task and
schedule definitions in Git-managed TOML. Skip missed schedules and report gaps.
These product requirements were confirmed before this interview.

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
  production deployment in `the deployment repository`. Implementation does not authorize
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

### P02: Preserve accepted work and stop on uncertainty

- **Type and status:** Principle; uncertainty handling and planned maintenance
  accepted by the user. The admission and lifecycle contracts are in
  [execution.md](temporary/execution.md#admission-contract-decisions); launch and lifecycle
  mechanisms still need specification. Manual admission transactions are implemented.
- **Rule:** If external work is uncertain, preserve queued runs and block new
  starts until the administrator reviews and acknowledges it. Do not
  automatically retry interrupted runs. Local process cleanup is not proof
  that external work stopped. For planned restart or upgrade, pause new starts
  and wait for active runs to finish before restart. Unexpected shutdown uses
  recovery and review. A forced maintenance stop must not silently bypass this
  policy. Acknowledgement records the affected runs, evidence, remaining
  uncertainty, and reason to resume. The administrator can explicitly accept
  remaining risk; acknowledgement does not prove external completion or change
  a run's result. Other dispatch blocks still apply.
- **Reason and cost:** Avoid unsafe overlap and repeated external changes.
  Review can delay the queue; draining can delay maintenance. Continuing after
  local cleanup would reduce delay but would not resolve external uncertainty.
- **Check:** Inject failure around task launch and completion. Confirm queued
  work remains blocked, interrupted work is not retried, and planned maintenance
  waits for active work. Define the acknowledgement record before implementation.
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
  Confirm that both retain their original selection. Define runtime identity,
  retention, writable paths, and missing-runtime handling before execution.
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
  retained review output until
  acknowledgement; reserve log allowance before dispatch. Follow the accepted
  [retention contract](temporary/execution.md#retention-contract-decisions).
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
  RSS and latency figures remain proposed measurement targets, not accepted
  guarantees or measured results.

## Accepted operational controls

The user accepted these effects in the glossary interview on 2026-11-08:

- Schedule pause stops new scheduled requests and records known occurrences as
  skipped. Manual requests and queued runs remain eligible. Resume does not replay
  skipped occurrences. A known pause alone is not a schedule gap.
- Dispatch pause stops new starts without stopping active runs or closing admission.
- Admission closed rejects requests from all triggers. By itself, it does not stop
  queued runs from dispatching or stop active runs.

The controls can be combined; other safety blocks still apply. These decisions
accept only the pause meanings from the plan's P04 area. Reload, checkpoint, and
restore mechanisms remain open. Accepted restore behavior is specified in [the
execution contracts](temporary/execution.md#application-state-restore-contract).
See [the glossary](GLOSSARY.md).

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

These scenarios expose sensitive decisions: systemd permissions, the launch and
recording crash window, storage-independent stopping, runtime retention, and the
distinction between local cleanup and external completion. Manual admission is implemented and tested as a library. Process execution
is not implemented or verified.

## Coverage and open decisions

Policy is settled for purpose, ownership, recovery, fixed runtimes, storage
failure, resource defaults, operational controls, and restore review. Detailed
contracts are recorded in [execution.md](temporary/execution.md); they are
accepted future behavior, not implementation evidence.

[Remaining specifications and
evidence](temporary/implementation.md#remaining-specifications-and-evidence) cover
launch transactions, admission service integration, systemd integration, schedule checkpoints, record
formats, runtime layout, and retention/restore mechanisms. Performance
measurements, other metadata and buffer limits, host resource controls, and the
private access mechanism remain open. Library choices are made per component;
the configuration and admission dependencies are already selected.

License selection is a release gate. Workload compatibility, production access,
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

2026-11-08: Accept the glossary and execution contracts, now maintained in execution.md.
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

2026-11-08: The user selects a dedicated systemd user manager, confirms Debian
13 as the fleet baseline, and requests a replaceable supervision adapter for
manager-free tests. Target systemd 257. Real host verification, runtime and
output integration, and durable launch recovery remain required before the
admission store can dispatch. Transient service settings are implementation
mechanisms, not permission to weaken P02, P06, or P07.
