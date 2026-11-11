# Remaining acceptance cases

Status: A06/A07 are implemented but unrun on Docker. A08–A10 remain work in progress.
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

## Evidence and lifecycle

`remaining-A06-fixture/` contains the case store, configuration, work, output,
child-controller logs, startup proof, and case/host decisions. The independent
remote observation is `remaining-A06-target.json`. The root result includes each
implemented case. A missing or failed decision cannot produce an overall pass.

All created stores require stopped-writer confirmation before the credential
scan and raw copies. The scanner covers these stores along with all earlier
fixture roots. Target inspection and public logs are retained before teardown.
No private key is copied. Each case records its own result; prior results are
not rewritten to complete a later case.

Run the cases on Docker and inspect their complete evidence. The service loop,
scheduling, live reload, retention, UI, telemetry, production secrets, migration,
and deployment remain outside this work.

Before removal, transfer lasting fixture interfaces to ARCHITECTURE.md, preserve
useful evidence and review state, transfer open work, and update inbound links.
Follow the [temporary document lifecycle](README.md).
