# Rustward constraints and principles

Scope: Application architecture for the first version of Rustward.
Status: Mixed. Accepted decisions and proposals are marked below.
Decision source: Operator interview on 2026-11-07.
Parent guidance: [AGENTS.md](../AGENTS.md).
Design context: [Executor plan](plans/executor.md).

## Intent and use

Build a small task executor for one trusted administrator. Support generic
commands, manual requests, UTC cron schedules, and a web UI. Keep task and
schedule definitions in Git-managed YAML. Skip missed schedules and report gaps.
These product requirements were confirmed before this interview.

The main drivers are safe recovery, bounded resource use, fixed execution inputs,
and a small operational boundary. The administrator operates the service and
reviews uncertain work. Task owners remain responsible for external effects.

Follow accepted rules within scope. Proposals do not govern implementation.
Cite rule IDs in design reviews. Preserve IDs from the executor plan. The
decisions below replace conflicting proposals in that plan, particularly planned
shutdown in P02. Other plan proposals remain proposals. Scenario checks below
describe required evidence; they are not test results.

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
  browser requests. Select manager type and unit permissions before backend
  integration. Do not grant broad privileged unit creation as a shortcut.
- **Reason and cost:** Reduce custom supervision and deployment components.
  Availability depends on one host. Real execution tests need an isolated systemd
  host; a rootless development container cannot prove host integration.
- **Check:** Verify unit permissions, process cleanup, and concurrent run
  isolation. Verify proxy access enforcement and browser request protection.
- **Revisit:** A requirement for another platform, multiple administrators, or
  host failure tolerance changes this boundary.

### P02: Preserve accepted work and stop on uncertainty

- **Type and status:** Principle; uncertainty handling and planned maintenance
  accepted in this interview. The durable admission mechanism remains proposed
  in the executor plan.
- **Rule:** If external work is uncertain, preserve queued tasks and block new
  starts until the administrator reviews and acknowledges it. Do not
  automatically retry interrupted tasks. Local process cleanup is not proof
  that external work stopped. For planned restart or upgrade, pause new starts
  and wait for active tasks to finish before restart. Unexpected shutdown uses
  recovery and review. A forced maintenance stop must not silently bypass this
  policy.
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
- **Rule:** Start with one active task, a queue of 100 requests, a 10 MiB log
  limit per run, a 1 GiB aggregate retained-log limit, and 30-day retention.
  Make these limits configurable and enforce them. Metadata and transient
  buffers also need bounds; their values remain open.
- **Reason and cost:** Prevent task activity from exhausting service resources.
  Bounds can reject requests or remove retained output. Retention and aggregate
  limits need a defined cleanup policy before log implementation.
- **Check:** Exercise queue saturation, output flooding, and retention under
  load. Measure rather than infer memory and response-time behavior.
- **Revisit:** Change defaults using workload evidence. The plan's 64 MiB idle
  RSS and latency figures remain proposed measurement targets, not accepted
  guarantees or measured results.

## Scenario checks

| Source and trigger | Component and conditions | Required response and success measure |
| --- | --- | --- |
| P02: Service dies during external work | Recovery; a run is active and other requests are queued | Preserve queued requests; start none before acknowledgement; retry no interrupted task automatically |
| P02: Operator requests planned maintenance | Dispatch; a task is active | Pause new starts; finish active work before restart |
| P03: Prepared runtime changes | Queue and executor; old requests remain accepted | Old requests retain the original runtime; new requests can select the new runtime |
| P06: A write fails | State or log storage; a task is active | Close admission and dispatch; stop local work; require review if external work is uncertain |
| P07: Output exceeds 10 MiB | Log capture; default limits apply | Mark truncation, continue draining, and keep buffers and retained output bounded |
| Confirmed scope: Service misses cron times | Scheduler restarts after downtime | Execute no missed occurrence; report the gap and select a future occurrence |

These scenarios expose sensitive decisions: systemd permissions, the launch and
recording crash window, storage-independent stopping, runtime retention, and
the distinction between local cleanup and external completion. No executor
behavior is implemented or verified yet.

## Coverage and open decisions

The interview is complete for these architecture decisions. Detailed contracts
and unaccepted plan proposals remain implementation gates, not implicit defaults.

- **Purpose and ownership — settled:** Rust, generic tasks, one administrator,
  Git-managed definitions, and separate application and deployment repositories.
- **Correctness — settled at policy level:** Block uncertain work, preserve the
  queue, and do not retry interrupted tasks automatically. **Open:** YAML and
  argument mapping, state transitions, admission identity, and acknowledgement
  records. Define and review these before the related implementation.
- **Dependencies — settled at boundary level:** Linux/systemd, SQLite, and
  externally prepared runtimes. **Open:** Library choices, systemd baseline,
  manager and unit permissions, and runtime layout. Resolve before integration.
- **Performance — settled defaults, open measurements:** Queue, execution, and
  log defaults are accepted. Metadata bounds, host resource controls, retention
  interactions, and measurement workload need specification before acceptance.
- **Operations — settled policies, open mechanisms:** Private proxy access,
  drain before planned restart, and stop on storage failure. Select the exact
  access mechanism before HTTP integration. The plan's P04 pause/reload details
  and P05 restore policy remain proposals; review them before those features.
- **Evolution — open release gate:** Select the license before publishing
  releases. Review schema migration, backup consistency, and rollback before
  operational acceptance. Distributed execution and other scope extensions are
  not authorized by this document.
- **Migration — separate gate:** Review a sanitized actual Semaphore export
  before fixing workload compatibility. No live inventory has been verified.

Residual risks are external work that cannot be observed reliably, host-specific
unit permissions, single-host availability, and storage faults that prevent
durable fault records. Resolve mechanisms with focused specifications and
synthetic failure tests; do not treat scenario reasoning as execution evidence.

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

Ask about unresolved choices that can change affected rules. Reuse known
answers and respect explicit deferrals. Use ASD-STE100 Simplified Technical
English adapted to software terminology.

2026-11-07: Record operator decisions from the Gravity interview. Retain C01,
P01, P02, and P03 IDs. Replace the proposed stop-on-planned-shutdown policy
with drain-before-restart. Add P06 storage-failure policy and P07 resource
defaults. Leave other plan proposals unaccepted.
