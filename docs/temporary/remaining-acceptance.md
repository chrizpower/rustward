# Remaining acceptance cases

Status: A06–A10 are implemented but unrun on Docker.
The user reported A01–A05 passing with source revision
`1690669832608c67aa55d5d36734c33c73601ed9eb76d2a2609cad236e9d4e4e` and image
`sha256:9eab44b973b203a3ffac40a3da340ff65de95b9da1a10d9d312cb792a56e940f`.
The complete successful bundle has not been copied here. Docker is absent from
the devbox; synthetic checks do not pass these host cases.

Run `just acceptance-ssh` on the existing isolated Docker host. Each remaining
case has its own store, writable roots, case decision, readiness checks, and host
cleanup report. A fresh target uses a separate host pin under
`/run/rustward-credentials/remaining/`. Earlier pins, runs, queues, and review
blocks are preserved. The existing protected runtime is shared. No production
credentials, migration, deployment, or service loop are introduced.

## A06: Remote recovery

Admit a remote holding command and another queued run. A child controller starts
the first run. It records readiness only after Running and a live remote marker
with matching admitted run ID, typed inputs, PID, and process start time. The
parent kills that controller with SIGKILL, without a clean shutdown.

Recovery must retain Interruption, execution identity, queued work, one wrapper
start, and required review. Local cleanup must finish, capacity must be released,
and the wrapper must end. Recovered output must retain unknown historical counts
and cannot support Success. Dispatch must remain blocked after local cleanup.
Docker independently reads the remote marker after recovery; an alive or ended
process does not clear review.

## A07: Asynchronous work

An actual Ansible async command uses poll=0, followed by a separate synchronous
wait task. The controller is killed after the remote async marker appears.
Recovery must retain Interruption and local cleanup while the same remote PID
and start time remain alive. The fixture checks the review block, then records
one explicit acknowledgement note accepting the remaining external risk for
this disposable target. Matching acknowledgement retries return the same record.
The result stays Interruption and queued work is not started. Docker must also
observe the same async run alive. Other fixture stores' review blocks remain.

## A08: Runtime selection

Root setup publishes separate protected A and B runtime trees and records their
production manifest IDs. The source runtime used by other cases is not changed.
The same task ID has separate A/B configuration selections. A is admitted before
B is selected, matching retries still return A, and A must execute A's code.
While A is active at a confirmed remote holding marker, a new admission selects
B and stays queued until A completes. A fixed release message lets A finish;
then the new run must execute B's code and complete the independent B target proof.
The original snapshot, arguments, revisions, fingerprint, and runtime ID must
remain byte-for-byte unchanged while queued, active, and completed.

A third A run is admitted before root setup moves only this case's A tree out
of its catalog. Dispatch must fail before any program starts, with no B
substitution. The removed tree is retained under `removed-runtimes/` as evidence.
The original admission and completed A history must remain unchanged. These are
private fixture preparations, not a runtime selection UI or production migration.

## A09: Output limit

A private fixture capture limit of 65,536 bytes applies through the controller's
existing limit interface; it does not implement service-setting activation.
A remote command emits 1 MiB on each stream and writes a matching completion
proof. Ansible debug tasks emit both captured strings through the local playbook.
The run must finish with clean Success, synchronized observed counts above the
limit, retained bytes exactly at the limit, and recorded truncation. The reader
continues draining; truncation cannot close admission, stop work, or keep a slot.
The stderr marker need not be retained if stdout consumes the shared allowance.
Docker independently checks the remote completion proof and admitted run ID.

## A10: Log-storage failure

Root setup mounts a private 1 MiB tmpfs at this case's log root and fills
768 KiB with a ballast file. The state database, work, and evidence stay on the
normal filesystem. Only the disposable controller container receives this mount.
The task first confirms active remote work. An independent SQLite connection
then holds a write lock, and a fixed trigger makes the local wrapper emit 2 MiB.
This exhausts the log filesystem below the normal capture limit.

Without controller polling or another successful database write, the emergency
worker must close the storage gate and stop local work. A separate systemd
adapter must confirm cleanup of the retained execution identity while the lock
is still held. The database must have no result at this point. After unlock,
new admission and dispatch must be refused, the active run must complete as
Interruption with failed output and required review, and the queued run must
remain unstarted. The gate stays closed and local capacity is released.

The host report must confirm that the private log filesystem has zero free
bytes. Docker independently reads the matching remote marker; it cannot use
local cleanup as proof of remote completion. The mount remains through evidence
copy and disappears with the container. No host or database disk is filled.

## Evidence and lifecycle

`remaining-A06-fixture/` through `remaining-A10-fixture/` contain the case
stores, configuration, work, output, and case/host decisions. Recovery cases also
retain child-controller logs and startup proofs. Independent target observations
use `remaining-A06-target.json` through `remaining-A10-target.json`. The root
result includes A01–A10. A missing or failed decision cannot produce an overall pass.

All created stores require stopped-writer confirmation before the credential
scan and raw copies. The scanner covers these stores along with all earlier
fixture roots. Target inspection and public logs are retained before teardown.
No private key is copied. Each case records its own result; prior results are
not rewritten to complete a later case.

## Devbox checks

`just check` passes: 214 Rust tests, 59 Python tests, and 95.71% Rust line
coverage. Three opt-in host tests are ignored in normal checks. An explicit
Docker attempt reports A01–A10 not run because Docker is not installed.
Check logs and the blocked result are under
`$HOME/artifacts/rustward/a06-a10-20261111/`.

Direct local Ansible smoke checks confirmed that async work survived termination
of its local process group and that the flood playbook emitted over 2 MiB.
These checks did not use Rustward, systemd, or Docker and do not pass A07 or A09.

Run the cases on Docker and inspect their complete evidence. The service loop,
scheduling, live reload, retention, UI, telemetry, production secrets, migration,
and deployment remain outside this work.

Before removal, transfer lasting fixture interfaces to ARCHITECTURE.md, preserve
useful evidence and review state, transfer open work, and update inbound links.
Follow the [temporary document lifecycle](README.md).
