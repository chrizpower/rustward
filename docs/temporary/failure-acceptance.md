# A04 failure acceptance

Status: The copied bundle confirms A04 task failure and unreachable-target
results, including retained Failure and captured output.
See the [evidence review](acceptance-review.md) for the current bundle and
remaining A10 output export gap. Docker is absent from the devbox.

## Run and scope

Use the same isolated Docker host and command as A03:

```sh
just acceptance-ssh
```

The runner requires A01/A02, local recovery, the three legacy host tests, and A03
to pass before A04. It uses the existing private internal network, fixture-only
key files, pinned host key, verified runtime, dedicated non-root manager, output
capture, and generic controller. Production credentials, inventory, deployment,
and changes to execution policy remain outside this implementation.

A04 has two separate admitted runs. They use the existing SSH fixture database
and new run IDs. Each admission checks a matching retry with the same key and
payload. Each wrapper must start once. No failed run is resubmitted, and stored
A03 results and identities are unchanged. The local recovery fixture keeps its
separate database, queued run, and review block.

## Deliberate task failure

The first run uses `remote-failure`. It passes the same string, integer, and
boolean inputs as A03 to a separate playbook. A remote command uses Python to
create `/home/rustward-target/failure.json` with those values, prints
`A04_EXPECTED_REMOTE_FAILURE`, and exits with code 23. Exclusive file creation
makes a repeated remote command fail to produce the intended proof. No shell
interprets the input values.

Ansible must exit with code 2, and Rustward must retain Completed/Failure with
that exit evidence. The runner then reads the proof file through Docker and
checks its values independently before it stops the target. A known task failure
is the expected workload result; it does not count as an acceptance-test failure
when all evidence checks pass.

## Unreachable target

After the task failure completes, Docker stops the target. The runner requires
Docker inspection to report `Running=false` and `Pid=0` before it admits
`remote-unreachable`. It does not use a sleep as proof that the target stopped.
A stop or inspection failure prevents the unreachable run from starting.

This second run uses the original A03 playbook, target address, port 2222,
credential file, and strict host-key options. No alternative host, port, or key
is substituted. Ansible must report an SSH connection failure and UNREACHABLE,
and exit with code 4. Rustward must retain Completed/Failure. This case covers a
connection failure before remote work; it does not establish remote cancellation
or asynchronous cleanup behavior.

## Completion and evidence

Both runs require the exact expected exit, retained execution identity, confirmed
local cleanup, no output failure or review block, synchronized non-recovered
capture, correct byte accounting, no truncation, both wrapper output markers,
expected typed inputs, and proof that the wrapper process ended. Controller
capacity must remain held while the run is unfinished and return to zero after
completion. The Failure result must survive reopening the database.

A04 adds these records to the existing evidence folder:

- `ssh-fixture/evidence/A04-task-failure.json` and `A04-unreachable.json`: separate
  decisions with run IDs, state, exit, cleanup, output, inputs, and runtime IDs.
- Separate admission, manager-readiness, and host-runner records for each run.
- `ssh-fixture/evidence/A04.json`: a pass only after both case decisions and both
  host cleanup records pass. The existing A03 `result.json` is preserved.
- `failure-target-check.json`: the independently checked remote failure proof.
- `target-stopped.json`: Docker's stopped-target state before the second run.

The root result includes A04. An unfinished case stays incomplete; a failed
subcase reports failure. Missing case, target, or cleanup evidence prevents a
pass. The original credential scan covers both additional runs' retained state
and output before either fixture is copied. No credential bytes enter inputs,
arguments, environment variables, or copied fixture roots. The target is never
restarted to repeat a task. A05 uses a new target container with its own host pin.

## Verification and remaining work

Focused tests check the accepted task layout and argument generation, expected
Failure versus Success, wrong or missing exit evidence, incomplete cleanup,
review blocks, output faults, and truncation. Python tests check fixture modes,
unchanged strict SSH policy, target-stop confirmation, ordering, required reports,
partial failures, and refusal to overwrite an existing A04 decision.

Direct smoke checks use Ansible 2.19.0 with a local synthetic target for the
failure playbook and a reserved non-listening loopback port for the unreachable
case. They observe exit codes 2 and 4 and check useful output and proof values.
They do not use Rustward or systemd, and do not pass A04. Evidence and check logs
are under `$HOME/artifacts/rustward/a04-failures-20261111/`.

`just check` passes: 209 Rust tests and 38 Python tests, with 95.71% Rust line
coverage. Three real-host tests are ignored in normal checks. The explicit Docker
attempt is blocked because Docker is not installed in the devbox.

The copied review confirms A01–A09. Resolve the [A10 export gap](acceptance-review.md)
and review the corrected Docker evidence. The service loop, scheduling, live reload, retention, UI,
production secret configuration, workload migration, and deployment remain open.

Before removal, transfer lasting fixture interfaces to ARCHITECTURE.md, preserve
useful evidence, transfer open work, and update inbound links. Follow the
[temporary document lifecycle](README.md).
