# Rustward requirements

Status: Temporary reference. Preserve requirement IDs, acceptance criteria,
accepted scope, and unresolved proposals in ARCHITECTURE.md or GRAVITY.md before
removing this file. Update inbound links after transfer. See [folder guidance](README.md).

This reference preserves product requirement IDs and acceptance scenarios from
the original executor plan. Partial acceptance does not accept the rest of a
requirement. Unaccepted clauses and proposed performance targets remain proposals.
Use [Gravity](../GRAVITY.md) for accepted architecture rules,
[execution.md](execution.md) for accepted execution contracts, and
[configuration.md](configuration.md) for implemented schema behavior.

## Purpose and authority

Replace Semaphore with Rustward, a small generic task executor written in Rust.
Ansible is the first workload. Confirmed scope is Rust, manual requests, UTC
cron, Git-managed YAML definitions, a web UI, one trusted administrator, and
skipping missed occurrences while reporting gaps. Application code belongs in
`chrizpower/rustward`; deployment belongs in `the deployment repository`.

Configuration validation, argument generation, durable manual admission, and
queued cancellation are implemented. Execution,
scheduling, and operational behavior remain unimplemented. This reference does not
authorize live execution, production deployment, or schedule migration.

## Status and authority

| Area | Current authority |
| --- | --- |
| C01, P01, P02, P03 | Accepted constraints and principles in Gravity; detailed execution contracts in execution.md |
| P02 planned shutdown | Drain active runs before planned restart; do not apply the original stop-on-shutdown proposal |
| F02 and P02 request keys | Manual admission library implemented; web UI and service integration remain future work. Rustward-issued permanent manual keys have a narrow P07 metadata exception |
| F06–F08 scheduling | Accepted occurrence identity, cron day matching, and one queued run per schedule in execution.md |
| P04 controls | Accepted independent pause/admission controls; reload and checkpoint mechanisms remain open |
| P05 restore | Accepted blocked restore and new key generation; backup and restore mechanisms remain open |
| N05 limits and storage failures | Accepted P06/P07 defaults, permanent-key exception, and retention contract |
| F11 queued cancellation | Implemented in the library with separate phase and result; active cancellation and timeouts remain future work |
| F01/F03/F04/F15 configuration and inputs | Implemented version 1 contract in configuration.md; remaining execution/UI portions are not implied |
| N02/N03 performance | Proposed targets only; no measurements or guarantees |

Use requirement and rule IDs in reviews. Read current contracts before using a
requirement below; partial acceptance does not accept the rest of a requirement.

## Functional requirements

| ID | Requirement | Acceptance condition |
| --- | --- | --- |
| F01 | Load versioned YAML task and schedule definitions. Each task has a stable ID, label, executable, argument list, working directory, and input schema. | Reject duplicate IDs, unknown keys, invalid references, and invalid schedules before activation. A failed reload leaves the previous valid configuration active. |
| F02 | Run a defined task from the web UI with optional inputs. | Accept a Rustward-issued manual request key and return the same durable run ID for retries with the same payload; reject reuse with a different payload. Refreshing or closing the page does not stop the run. Show a clear error when the request is rejected. |
| F03 | Support required inputs, defaults, strings, integers, booleans, and explicit choice lists. | Validate on the server for both manual and scheduled requests. Reject undeclared inputs and values outside the schema. |
| F04 | Execute a program with separate arguments and a controlled environment. | Inputs remain data. Never insert them into a shell command string. Scripts use an explicit interpreter or executable. |
| F05 | Run Ansible through ordinary task definitions and an optional small wrapper. | A synthetic playbook receives the selected inventory, limit, and extra variables correctly. A second task runs a non-Ansible command through the same execution path. |
| F06 | Support five-field cron expressions in UTC. Schedules reference a task and fixed, validated inputs. | Show enabled state, last occurrence, next occurrence, and related runs. Test month boundaries and day-of-month/day-of-week semantics. Document the selected cron semantics. |
| F07 | Skip missed occurrences and report schedule gaps. | Restart after downtime creates no catch-up runs. Retain the affected schedule, interval, and missed count when known; mark uncertain intervals explicitly. |
| F08 | Use one shared queue for manual and scheduled requests, with a configurable execution limit. | Default to one active run globally. Bound the queue and report rejected requests. Coalesce further occurrences while the same schedule already has a queued request, and report them as skipped. |
| F09 | Preserve run state and execution identity. | Store task/configuration identity, validated non-secret inputs, trigger, scheduled time if present, timestamps, exit status, and reason for failure or interruption. |
| F10 | Show live output and retained history. | Display stdout and stderr, the accepted run phases and results, and a clear truncation notice. Reconnect to a run without restarting it. Do not promise a total order between separate output streams. |
| F11 | Support cancellation and per-task timeouts. | Stop queued requests or stop the run’s systemd unit. After a configured grace period, kill remaining processes in that run’s cgroup. Do not release its execution slot until local cleanup is confirmed. Distinguish failure, cancellation, timeout, and interruption. |
| F12 | Recover conservatively after process or host failure. | Mark uncertain active runs interrupted. Cancellation, timeout, and shutdown also block dispatch when external work remains uncertain. Do not rerun automatically. Block further execution until the administrator acknowledges uncertain external work. Preserve accepted queued requests under Gravity P02. |
| F13 | Send failure alerts through the established mail relay. | Alert on failure, timeout, interruption, and schedule gaps. Record notification failure separately from task failure. Batch gap reports after restart. |
| F14 | Support an operational pause for schedules. | A global schedule pause stops new scheduled requests and reports the paused interval. Accepted queued requests can still start, and manual requests remain available. A separate dispatch pause prevents all queued work from starting without stopping active runs. Resume does not replay skipped occurrences. Task and schedule editing remains in Git. |
| F15 | Provide configuration validation and service diagnostics. | A CLI validation command needs no live hosts or production secrets. Health, readiness, and bounded metrics expose scheduler/storage failures without secrets. |

## Non-functional requirements

| ID | Requirement | Proposed measure or verification |
| --- | --- | --- |
| N01 | Small deployment | One native Rust service with embedded web assets and local SQLite storage. No external database, broker, custom worker service, or frontend server. Existing systemd infrastructure manages per-run units; this does not require a separate application daemon. Ansible and other task programs are separate runtime dependencies. |
| N02 | Low overhead | Target idle RSS at or below 64 MiB on the selected Linux host with 100 tasks, 100 schedules, and 10,000 retained run records. Exclude child processes and filesystem cache. Record actual results before acceptance. |
| N03 | Responsive control path | Target p95 request acceptance below 250 ms and local process start below 1 second when a slot is free. Target visible output delay below 1 second. Measure on a stated host under the agreed workload; exclude task setup and execution time. |
| N04 | Durable state | Acknowledge a request only after durable storage. A storage error prevents new execution and changes readiness. Test forced termination around acceptance, dispatch, and completion. |
| N05 | Bounded resource use | Propose queue limit 100, log limit 10 MiB per run, retention 30 days, and aggregate retained-log limit 1 GiB. Make limits configurable. Keep draining output after truncation; do not block a child on a full log. Bound metadata and transient buffers too. Configure per-run memory, process-count, and CPU controls through systemd, and preserve host capacity for the control service. Define log-storage failure behavior before implementation; stop new admission and dispatch, report incomplete output, and apply the task’s interruption policy to active work. |
| N06 | Restricted service access | Run as a dedicated unprivileged account, bind to loopback, and use private HTTPS through Nginx. Require one administrator access check at the proxy. No user database, roles, or public registration. |
| N07 | Protected browser actions | Authenticate UI, API, output streams, and diagnostics as appropriate. Protect state-changing requests against CSRF, reject cross-origin access, and escape task output before display. |
| N08 | Secret handling | Use protected references to the established secret store. Keep secrets out of task definitions, stored inputs, URLs, diagnostics, and command previews. Test with synthetic secrets. Task programs remain responsible for safe output, including Ansible no_log; generic log filtering cannot guarantee secret removal. |
| N09 | Reproducible runs | Record the application version, task definition revision/hash, prepared repository revision, and runtime identity. A queued run uses its accepted definition and prepared runtime, or fails explicitly if they are unavailable. |
| N10 | Recoverable operations | Document persistent paths, backup, isolated restore, release rollback, schema compatibility, and shutdown behavior. Prove restore with synthetic history and definitions before migration. |
| N11 | Maintainable implementation | Use stable Rust, a committed lockfile, a small dependency set, format/lint checks, and focused tests for execution, scheduling, recovery, and HTTP access. Prefer established libraries over custom parsers or cryptography. |
| N12 | Supported platform | Support Linux with systemd first. Validate the production CPU architecture and Linux baseline before release. Development tests must not need fleet access or private Vault credentials. |

P07 accepts the queue and log defaults listed under N05, with the permanent-key
exception. N02/N03 performance figures remain proposed measurement targets. None
of these figures is evidence of measured behavior.

## First-version exclusions

These exclusions retain their original proposal status except where explicitly
accepted by Gravity or the execution contracts.

- Distributed workers, high availability, remote agents, and multi-tenancy.
- User management, per-task permissions, SSO, and an internal credential vault.
- Workflow graphs, task dependencies, automatic retries, and artifact browsers.
- Automatic discovery of inventories/playbooks and structured Ansible host/task
  result views. Preserve ordinary output first.
- Built-in container orchestration, Git hosting, and per-run dependency installs.
- Interactive terminal sessions or prompts. Unattended tasks must receive all
  required inputs and credential references before execution.
- Browser editing of commands or schedules and arbitrary shell entry.
- A plugin framework or separate Ansible execution engine.

## Acceptance scenarios

These checks use selected QAW and ATAM practices. They describe required
evidence, not test results.

| Source / trigger | Conditions and component | Expected response and measure |
| --- | --- | --- |
| User requirement: run a playbook now | No run is active; a prepared Ansible runtime exists | Accept the request durably, start within N03, stream output, and retain the result and revision. |
| Generic scope: run a script | No Ansible configuration is present | Execute through the same core with no Ansible dependency in the service. |
| User decision: offline during a scheduled backup | Restart after multiple cron occurrences | Start no missed work; report the gap and show the next future occurrence. |
| Backup runbook: overlapping maintenance requests | Global execution limit is one | Queue within the configured bound; never overlap local executions. Keep remote workload locks. |
| Operational failure: service dies during maintenance | Local completion and remote state are uncertain | Mark interrupted, clean local children, alert, and block dispatch until acknowledgement. Never automatically rerun. |
| Security requirement: input contains shell syntax | Administrator submits a string through the form | Pass it as data or reject it by schema; execute no extra command. |
| Resource requirement: command writes unlimited output | Log limit is reached while the task runs | Mark truncation, continue draining, and keep memory/storage within configured limits. |
| Recovery requirement: host is rebuilt | Configuration, credentials, and supported state backup are available | Restore with execution disabled; reconcile restored queued/active work, report the outage gap, validate runtime, and enable only one scheduler after explicit release. |

Cancellation cannot prove that a remote operation stopped or restored a service.
The UI and runbook must state this limit. For backup cancellation, confirm the
affected service and remote lock state before acknowledging interruption.

## Implementation and migration

Implementation sequencing and unresolved mechanism proposals are tracked in
[temporary implementation work](implementation.md). One-time workload
inventory, operational context, migration safeguards, and cutover evidence are
tracked in [migration preparation](migration.md). They do not authorize
production changes.

## Maintenance

Keep requirement IDs stable and record acceptance explicitly. Follow Gravity's
change and review guidance. Do not infer implementation from acceptance scenarios;
keep current implementation status in the architecture and interface references.
The 2026-11-08 consolidation removed duplicate architecture rules and superseded
sequencing without accepting additional proposals or changing requirements.
