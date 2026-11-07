# Implementation handoff

Begin with phase 0 of [the reviewed plan](plans/executor.md). This bootstrap
contains no scheduler, database, API, or execution backend.

## Confirmed scope

- Rust; framework name Rustward; generic command execution with Ansible first.
- Manual execution and cron schedules; YAML definitions in Git.
- Web UI for inputs, execution, logs, and schedule status.
- One trusted administrator; skip missed schedules and report gaps.
- Existing Linux infrastructure is permitted. Repository bootstrap is authorized.

## Proposed design

Use one Rust service, embedded web assets, local SQLite, bounded logs, and
systemd transient services with a separate cgroup for each run. Keep admission
and occurrence identity durable. Never automatically retry uncertain work.
Use separate schedule and dispatch pauses. Restore with execution disabled.
Prepare immutable repositories and runtimes outside execution. Preserve runtime
references for queued work. See the plan for requirement IDs and failure cases.

## First implementation task

1. Review the actual Semaphore task export with the operator. Do not assume
   that runbooks list every task. Keep secrets out of exports and this repository.
2. Select the license, Linux/systemd baseline, access method, system or user
   manager, and narrowly scoped unit permissions. Define the prepared-runtime
   contract, numeric limits, and storage-failure behavior.
3. Define YAML examples, input-to-argument mapping, run states, and transitions.
   Cover admission retries, cancellation, timeout, dispatch pause, and restore.
4. Select stable Rust libraries and record the decisions. Implement phase 1
   with synthetic commands before adding the scheduler or web UI.

Phase 1 evidence must cover failed exec, crash windows, duplicate submissions,
queue bounds, output flooding, disk failure, detached children, concurrent run
isolation, and uncertain external effects. Test systemd on an isolated Linux
system with the selected manager. Do not expose the production host bus or grant
privileged access to the devbox merely to make integration tests pass.

## Operational context

the deployment repository owns the native service role, private Nginx access, secret references,
SSH host trust, managed agents, metrics, backups, and migration from Semaphore.
The source plan links the authoritative runbooks. Their configuration does not
prove live state. Read the current versions before deployment work.

No live tasks have been exported or migrated by this bootstrap. No production
runtime, access mechanism, or systemd permissions have been validated.
