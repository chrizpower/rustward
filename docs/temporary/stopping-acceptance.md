# A05 stopping acceptance

Status: Implemented, not run on Docker. The user requested A05 on 2026-11-11
after reporting A01–A04 passing. The latest complete successful bundle has not
been copied here. Docker is absent from the devbox. Synthetic tests and direct
local Ansible checks do not pass A05.

## Run and scope

Use the same isolated Docker host and command:

```sh
just acceptance-ssh
```

The command runs A01–A05. The timeout case waits for a real 60-second limit from
dispatch; it does not advance a test clock or reset the limit after startup.
This fixture limit does not change the one-hour product default. Startup evidence
must arrive within 45 seconds, before that timeout. Fixture cleanup grace is
two seconds. The command can take longer while it builds images or prepares the
runtime.

A05 adds queued cancellation, active cancellation, and active timeout. All runs
use the existing manual admission, controller, systemd supervision, and capture
path. There is no service loop or SSH-specific branch in the execution core.
Production inventory, credentials, migration, and deployment remain outside
this implementation.

## Fresh target and fixed selection

A04 leaves its SSH target stopped. A05 creates a new target container from the
same fixed target image on the same private internal network. It does not restart
the old target or resubmit an old run. Docker's setup channel supplies the new
public host key and address. Its separate known-hosts and connection files live
under `/run/rustward-credentials/stopping/`; the earlier A03/A04 files are unchanged.
The existing fresh fixture client key stays outside every copied fixture root.

SSH still requires strict host-key checks, batch mode, the fixed identity file,
no agent, no forwarding, and no control sockets. Each A05 fixture repeats the
correct-key and wrong-key probes and the installed Ansible SSH/SFTP command
checks. Only read-only readiness probes can repeat; no workload is started again.

The remote holding command creates a case-specific marker with exclusive file
creation. It records the admitted run ID, typed input values, PID, and Linux
process start time before it holds the synchronous command open. PID and start
time distinguish a live process from PID reuse. A zombie counts as ended. A
five-minute watchdog bounds abandoned fixture work; it cannot make A05 pass.
The marker is evidence of one process, not proof that all remote effects ended.

The native test reads that marker through a separate pinned, read-only SSH
connection while it polls the controller. It requires the expected live process,
inputs, case, and admitted run ID before it proceeds. An absent marker is not
startup proof. Invalid or late evidence fails the test. No sleep duration is used
as proof that remote work started.

## Three separate run checks

Queued cancellation is tested first. The test admits `remote-queued`, verifies a
matching submission retry, cancels it before claim, and repeats cancellation to
check idempotence. It requires Completed/Cancellation, no launch record, no work
or log directory, no active slot, and an empty dispatch queue. Docker checks that
the queued case's remote marker is absent before and after A05.

Active cancellation admits `remote-cancel` and waits for confirmed remote
startup. `cancel_active` must record Cancellation. Capacity must still be held
immediately after that decision and while the run is unfinished. Completion
requires local cleanup and output handling; it does not prove remote completion.

Active timeout admits `remote-timeout` in a separate fixture store. It waits for
the same startup evidence, then continues polling with the real monotonic clock.
Only the controller's configured timeout initiates this stop. The final result
must remain Timeout, with at least 60 seconds measured since dispatch.

Both active cases require retained execution identity and exit evidence,
confirmed local cleanup, the expected stop reason, required external review,
no output failure, synchronized non-recovered capture, correct byte accounting,
no truncation, typed inputs, one wrapper start, and proof that the wrapper ended.
An earlier Failure, Interruption, or Success cannot be rewritten to make the
case pass. The existing first-result rules still apply.

## Preserve external review and history

Cancellation and timeout of launched work conservatively require review. After
each case completes, the test admits another run and requires dispatch to return
ReviewRequired while local capacity is free. It reopens the controller and checks
that the result, identity, output record, review block, and queued work remain.
It never acknowledges a review or changes a completed result.

Cancellation and timeout use separate databases, work roots, log roots, and
evidence roots. This prevents one case from clearing another case's review block
to continue. Both stores retain their blocked queued run. The local recovery
fixture and A03/A04 history are also unchanged.

The two stores reuse the original protected SSH runtime and host catalogs rather
than copying code or changing runtime IDs. Private fixture metadata has an optional
absolute `catalog_root`; when absent, old fixtures still use their own root.
Service TOML and admission encodings do not change. Each new task keeps its
original absolute executable and working-directory selection. Source revision
and execution UID must match before shared preparation proceeds.

Docker reads each holding marker again after local completion. The observed
process can be alive or ended; either observation is retained without clearing
review. Its run ID must match the corresponding native case report. Final test
teardown removes the disposable target. This teardown is not an administrator
acknowledgement and does not rewrite stored review or results.

## Evidence and verification

A05 adds:

- `stopping-cancel-fixture/`: the queued and active cancellation database,
  configuration, work, output, case decisions, startup proof, and host checks.
- `stopping-timeout-fixture/`: the timeout database and corresponding evidence.
- `ssh-fixture/evidence/A05.json`: a pass only after all three native decisions
  and both host cleanup records pass. Existing A03/A04 reports are preserved.
- `stopping-target-metadata.json`: the fresh target's public key, endpoint, and
  package versions. Its Docker run command retains the fixed image ID.
- `stopping-cancel-target.json` and `stopping-timeout-target.json`: independent
  post-stop process observations, including the matching admitted run IDs.
- `stopping-queued-before-target.json` and `stopping-queued-after-target.json`:
  independent absence checks for the never-started queued work.

The host runner records `writers_stopped=true` only after the native process
ended and dedicated-manager cleanup succeeded. Export requires this evidence
for each created A05 store, including after a workload-test failure. A missing
confirmation or failed cleanup refuses raw export and retains the controller.
The outer Docker deadline allows the inner start, probe, execution, and cleanup
deadlines to finish.

The credential scan covers all four fixture roots before raw copies. A partial
case, failed cleanup, missing report, wrong remote run ID, or failed scan cannot
produce an overall pass. Public target logs and inspection data are collected
before the fresh target is removed. No private key is copied into these roots.

Focused tests cover retained stopping results, cleanup and review requirements,
remote identity and input checks, PID reuse and zombies, exclusive marker
creation, shared runtime selection, the 60-second setting, separate host pins,
cleanup failures, required reports, wrong run IDs, and partial evidence. Direct
Ansible 2.19.0 smoke checks execute the holding playbook against a local synthetic
target and check both stopping modes' SSH/SFTP commands. These checks do not
use Rustward or systemd and do not establish remote cleanup or pass A05.
Evidence and check logs are under
`$HOME/artifacts/rustward/a05-stopping-20261111/`.

`just check` passes: 211 Rust tests and 51 Python tests, with 95.74% Rust line
coverage. Three real-host tests are ignored in normal checks. The explicit
Docker attempt reports A01–A05 not run because Docker is absent from the devbox.

## Remaining work and removal

Run A05 on Docker and review the complete copied evidence. Remote recovery A06
is next, followed by A07–A10. The service loop, scheduler execution, live reload,
retention, UI, telemetry, production secret configuration, migration, and
deployment remain outside this implementation.

Before removal, transfer lasting fixture interfaces to ARCHITECTURE.md, preserve
useful evidence and review state, transfer open work, and update inbound links.
Follow the [temporary document lifecycle](README.md).
