# Migration preparation

Status: Temporary preparation for replacement of the existing scheduler. No live tasks have
been exported or migrated, and no production host state has been verified.
This document does not authorize exports, live execution, migration, or deployment.
Production work belongs in the deployment repository.

## Completion and removal

Before deleting this file, preserve applicable operating and recovery procedures
and acceptance evidence in the deployment repository through separately authorized work. Lasting
application direction is in [GRAVITY.md](../GRAVITY.md). The
[requirements](requirements.md) describe the current release and workload gates. Delete this file after migration acceptance or
explicit cancellation. Remove the temporary folder if it is then empty.

## Acceptance gates

These gates retain the original migration sequence and proposed evidence.
The [acceptance suite](acceptance.md) covers the synthetic cases in phase 4. The isolated pilot rehearsal checks monitoring and
recovery before phase 5. Use `just pilot-test` for synthetic alert delivery and
`just acceptance-container` for the local systemd restore rehearsal. Workload
review, live monitoring, and cutover gates remain open.

| Phase | Work | Exit evidence |
| --- | --- | --- |
| 4. Ansible acceptance | Prepare pinned controller/dependencies and a synthetic repository. Add task definitions for playbooks and a generic script. Test Vault, SSH agents, extra-variable serialization, exit codes, and dependency updates. | Use synthetic credentials and loopback SSH only. Prove credentials do not enter stored inputs or previews. Verify output safety and known-host behavior. |
| 5. Deployment preparation | Add the native service role, inventory selection, release verification, private proxy, firewall, metrics/log integration, bootstrap, backup/restore, and rollback procedures. | Relevant repository checks pass. Isolated restore and rebuild fixtures pass, including stale queued work after restore. Verify native SSH access before live use. |
| 6. Operator-approved pilot and cutover | Refresh the initial scheduler export and check for changes. Pilot harmless tasks. Check real alerts and runtime readiness. Migrate schedules in controlled groups with one active owner. | Recorded manual and scheduled results, gap report, alert delivery, resource measurements, rollback rehearsal, and operator acceptance. |

## Migration safeguards

The live task/schedule export is still required. Runbooks are not a complete
export. Include logging, mail maintenance, configuration, and other manual tasks
when checking the actual list.

Preserve the current scheduler state and a usable rollback path during the pilot.
Before enabling each new schedule, disable its old owner. Keep duplicate timers off
unless a separate design change explicitly transfers ownership. On rollback,
pause new schedule admission and dispatch, reconcile active/queued work, then
restore the old owner.

Review the native service account's route to each target. Do not
copy the existing container source exception to unrelated networks or disable
SSH host verification to make the replacement work. Test alerts through the
existing relay and preserve independent backup freshness monitoring.

Do not remove the existing scheduler or its database until migration acceptance. Retain the
database service if other applications use it. This document makes no live changes.
