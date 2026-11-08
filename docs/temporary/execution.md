# Execution contracts

Status: Temporary reference for contracts accepted on 2026-11-08. Manual
admission and queued cancellation are implemented as a library. Other lifecycle
transitions, scheduling, restore, and execution integration are not implemented.
A separate [supervision adapter](../ARCHITECTURE.md#supervision-interface-and-systemd-adapter)
is implemented and tested with synthetic manager responses. It is not connected
to the admission store. [Per-run bounded capture](capture.md) is also implemented;
aggregate reservations, retention, and service-wide failure handling remain future work. Absorb lasting behavior into ARCHITECTURE.md, policies and exceptions into
GRAVITY.md, and domain terms into GLOSSARY.md before removing this file. Preserve
all contracts, open decisions, and implementation status, then update inbound
links. See [folder guidance](README.md).

Follow [Gravity](../GRAVITY.md) for architecture policy and [the glossary](../GLOSSARY.md)
for terms. [Configuration](configuration.md) describes the implemented schema.
[Requirements](requirements.md) preserves requirement IDs and unaccepted proposals.
Remaining engineering work is tracked in the [temporary implementation handoff](implementation.md).

## Admission contract decisions

Manual request keys are unique across one Rustward installation. Repeating an
admitted key with the same task and submitted inputs returns the existing run
ID. Reusing that key with a different task or submitted inputs is rejected. A
fresh execution uses a fresh key. Scheduled occurrence identity is separate.

Manual request keys do not expire. Keep a compact admission record indefinitely
for each admitted key, including its request fingerprint and original run ID.
There is no fixed storage ceiling for these records. This is a user-authorized
exception to P07 metadata bounds, accepted on 2026-11-08. It replaces the
earlier bounded retry-window decision. Other metadata remains subject to bounds.
Storage failure still invokes P06; permanent records do not permit execution
without durable state. Monitor storage growth.

Rustward is the issuer of manual request keys. Issuing a key does not create a
run or reserve queue capacity. The implemented key format and handling of issued but unadmitted keys are
specified in [the admission library contract](../ARCHITECTURE.md#manual-admission-library).
Issuance creates no permanent record.

After run history is deleted, a matching retry returns the original run ID and
an explicit already-admitted/history-removed response. It does not recreate the
run or imply success. Do not retain a final-result summary solely for retries.

Compare the original task ID and typed submitted input values before defaults.
Omitted values differ from explicitly supplied defaults. A matching admitted
retry returns the original run ID without validating the current task
definition, even if the task changed or was removed. Bind a key permanently only
at admission. A rejected request can be corrected or retried with the same key
and is evaluated again. Once admitted, its payload cannot change. Keep rejection
diagnostics bounded and separate from permanent admission records.

Canonical request encoding and issued-key verification are implemented; see the
admission library contract. Restore
behavior is defined in the application-state restore contract below. Permanent
key records do not by themselves prevent duplicate execution after restoring a
backup that lacks newer admissions.

## Run lifecycle contract decisions

Store phase and result separately. The implemented queued cancellation
transition stores `Completed` and `Cancellation` atomically, preserves the
admission record, and releases queue capacity. No process cleanup is needed
because the run has not dispatched. Repeated cancellation does not change the
record. See the [library and migration contract](../ARCHITECTURE.md#queued-cancellation-library).
The other lifecycle transitions below remain accepted future behavior:


| Phase | Meaning |
| --- | --- |
| Queued | Waiting for dispatch |
| Launching | Slot reserved; program start not yet confirmed |
| Running | Program start confirmed; exit not yet observed |
| Cleaning up | Launch or execution has ended, or stopping has begun; local cleanup is not yet confirmed |
| Completed | Cleanup confirmed and final result recorded |

Results distinguish success, failure, cancellation, timeout, and interruption.
Record stop reasons separately while cleanup is pending. Competing-event rules
are defined below; exact transactional recovery transitions still need tests.

The execution timeout starts at dispatch and includes launch and execution, not
queue waiting. Stopping uses a separate cleanup grace period; the slot remains
held until local cleanup is confirmed.

For competing normal exit, cancellation, and timeout, the first durably recorded
decision determines the result. A later exit code 0 does not replace an earlier
recorded cancellation or timeout. A stop request after a recorded exit reports
that execution already ended. Recovery uncertainty and P06 still apply; event
ordering cannot justify an unsupported durable result.

Program exit code 0 means success. Nonzero exit codes and unexpected signals
mean failure unless an earlier recorded stop reason applies. Tasks with other
exit conventions use an explicit wrapper; version 1 does not need configurable
success-code lists.

Use a separate systemd service and cgroup as each run's local process boundary.
After the main program exits, stop remaining children. Confirm cleanup only when
the unit has ended and its cgroup has no remaining processes. A missing main PID
alone is insufficient evidence. Tasks must not leave local daemons behind.
Integration tests must verify detached children, unit disappearance, and
concurrent-run isolation on the selected systemd host.

Use a configurable 30-second default cleanup grace period before force-killing
remaining local processes. Grace expiry is not proof of cleanup; retain the slot
until cleanup is confirmed.

## Scheduling contract decisions

Each schedule has a stable ID. Occurrence identity is schedule ID plus UTC due
time; record the schedule-definition revision separately. Editing a schedule
does not make an already handled due time eligible again. Updated definitions
apply to future occurrences.

Use five-field UTC cron. When both day-of-month and day-of-week are restricted,
either match is sufficient; a time matching both still has one occurrence.
Wildcard handling and supported syntax must be specified and tested with the
selected parser.

Allow at most one queued run per schedule. Further due occurrences are recorded
as skipped while that queued run remains. An active run may have one queued
successor. The shared queue limit still applies.

## Retention contract decisions

Protect retained output needed for unresolved external review until
acknowledgement. This protects captured evidence; it does not remove per-run
truncation limits. Protected output can prevent further execution when the
aggregate log allowance is exhausted.

Remove eligible completed-run logs oldest completion first when reclaiming
capacity, including before normal retention age if needed. Before dispatch,
reserve the run's full per-run allowance within the aggregate log limit. If
protected logs and existing reservations leave insufficient capacity, keep the
run queued and report the block. The reservation is accounting, not a guarantee
against filesystem failure; P06 remains applicable.

## Application-state restore contract

These decisions concern backups of Rustward's own application state, not backup
tasks executed by Rustward. The user accepted the behavior on 2026-11-08; backup
tooling and production deployment remain separate work.

Restore starts with admission and dispatch closed. Review restored queued and
active runs before any can execute: they may have completed after the backup. Do
not automatically replay them. Create a new manual-key issuer generation. Known
old keys identify their restored admission records; unknown keys from older
generations are rejected. Fresh requests use the new generation.

Restore must be an explicit procedure; arbitrary rollback of a database cannot
be assumed safe. Define backup consistency, generation verification, and the
per-run review record before implementing restore. Normal restarts do not by
themselves require a new generation. No separate admission ledger is required by
this decision.

## Prepared runtime contract decisions

Keep prepared code and dependencies immutable. Give each run a separate writable
work directory outside its runtime for run-local temporary and working data.
Runtime updates affect new admissions only; queued and active runs retain their
accepted selection. A missing selected runtime causes an explicit failure; never
substitute the latest runtime. This does not prevent a task's intended external
effects on its workload data.

Runtime manifest format, paths, ownership enforcement, and writable-directory
removal remain implementation details to specify before integration.

## Definition revision contract

Normalize validated definitions after applying schema defaults. An omitted field
and an explicit equivalent schema default have the same revision. Ignore
comments, formatting, and object-key order; preserve meaningful ordered lists
and typed values. Labels, argument order, and actual default-value changes
affect revisions. This refines the glossary's parsed-content definition.

Request comparison is different: compare submitted inputs before defaults so
that retries remain independent of later configuration changes. The admission library implements versioned SHA-256 request fingerprints and
normalized definition revisions; see [the encoding contract](../ARCHITECTURE.md#request-identity).
Future schema changes must preserve stored revision meanings.
