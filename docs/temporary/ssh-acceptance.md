# A03 SSH acceptance

Status: A03 ran in the copied Docker bundle `20261111T154745Z-3cc42b56` and
failed because Ansible generated a long SSH control-socket path. A01/A02, local
recovery, and the three legacy host tests passed. Host-key checks, credential
scanning, and evidence copies passed. The fixture fix needs a Docker rerun.
The user accepted fixture-only credentials on 2026-11-11. Docker is absent from
the devbox. Synthetic tests do not pass A03.

## Run and scope

Run on the same isolated Docker host used for the local acceptance suite:

```sh
just acceptance-ssh
```

For an explicit evidence folder:

```sh
python3 -I -B acceptance/container.py --remote --artifacts "$HOME/artifacts/rustward/a03-run"
```

Use a new folder. SSH mode creates a unique internal Docker network and starts
the controller on it. It first runs A01/A02 and local recovery with no target
peer. It then builds and starts a disposable SSH target on that same network.
The local-only command keeps `--network=none`. No host ports, source mounts,
Docker socket, production inventory, or production credentials are supplied.
The controller container remains privileged for systemd; use an isolated Docker
host as required by the [local runner](acceptance-runner.md).

A03 uses its own fixture root and database, `/opt/rustward-ssh-acceptance`.
It does not acknowledge, remove, or change the local recovery test's retained
review block or queued run. The ordinary generic controller admits the remote
playbook twice with one submission key, requires one run ID and one invocation,
and waits for Completed/Success with exit, cleanup, and synchronized output
proof. No SSH or Ansible branch is added to the execution core.

The playbook checks a string with spaces and quotes, integer 7, and boolean true.
It writes known JSON on the target, reads it, and checks its content. The runner
then reads that file through Docker and checks it independently. A process exit
code alone cannot pass A03.

## Credential and host-key rules

Root setup generates a new Ed25519 client key inside the controller container
before the local suite starts, so its export marker is available if A01 fails.
The key stays at `/run/rustward-credentials/client_key`, outside both exported
fixture roots. Only the execution UID can read it (mode 0400); the enclosing
folder is root-owned. The wrapper uses this fixed file path. Credential bytes
are not task inputs, command arguments, custom environment, or runtime files.
This is a test fixture, not a production secret interface.

The target generates its own host key under its tmpfs `/run`. Its public host
key is read through Docker's setup channel and written to a protected known-hosts
file. Network key discovery is not used. SSH requires strict host-key checking,
a fixed host alias, batch mode, the supplied identity only, and no agent,
password, keyboard authentication, multiplexing, or forwarding. It ignores
inherited client configuration with `-F /dev/null`. `ControlPath=none` explicitly
disables control sockets as well as `ControlMaster=no` and `ControlPersist=no`. See
[OpenSSH configuration](https://man.openbsd.org/ssh_config) and the
[Ansible SSH connection options](https://docs.ansible.com/projects/ansible-core/2.18/collections/ansible/builtin/ssh_connection.html).

Before admission, a read-only SSH probe with a wrong pinned host key must fail
with host-key verification evidence. A probe with the correct key must succeed.
Before admission, the pinned runtime also checks the installed Ansible SSH and
SFTP command builders with a deliberately long potential control-socket path.
Neither command may generate that path. OpenSSH `-G` must confirm the effective
strict host-key policy and disabled connection sharing. This check makes no
network connection and writes `ssh-command-check.json`.
Only readiness probes can repeat; an admitted task is never retried. The wrong
key check does not pass A04's unreachable-workload case.

The separate target runs ordinary sshd on port 2222, with public-key access only
for a synthetic non-root account. It disables password login, root login,
forwarding, and interactive terminals. Python is installed before execution.
The controller runtime copies SSH and SFTP binaries and pins their ELF host
dependencies through the existing manifest preparation path.

## Evidence and failure handling

Evidence uses the local runner's unique artifact folder. It adds:

- `ssh-fixture/`: A03 runtime/host manifests, source revision and package versions,
  task configuration, state database, work markers, captured output, and decisions.
- `target-image-id` and `target-metadata.json`: the fixed target image ID,
  public host key, endpoint, and SSH/Python package versions.
- `target-check.json`: the independently checked target values.
- `credential-scan.json`: the export decision, without credential contents.
- Docker command, output, and status records. Setup transfers only public keys
  through standard input; private keys are never transferred through Docker.

After writers and the dedicated manager stop, the runner scans every regular
file in both fixture roots, including SQLite files, configuration, inputs, and
output. It searches for a literal private-key marker of at least 64 bytes and
handles matches across read boundaries. It also checks file names. It rejects
symlinks, special files, and excluded entries without following or reading them.
The marker is selected from the generated private key and is not part of the
public key. This test checks that literal marker; it does not prove detection of
all possible encodings or fragments of a secret.

Both raw fixture copies require a successful scan. If scanning fails or cannot
finish, the runner reports failure and refuses raw fixture export. A safe scan
report can identify the affected file, with marker-bearing paths hidden. The
stopped controller is retained for review. Stored history is not rewritten or
redacted to make the scan pass. Missing target evidence, case evidence, or copied
evidence also prevents a pass. Containers and the network are removed only by
name for this attempt; there is no global cleanup.

## Devbox verification

`just check` passes, including formatting, Clippy, file-size and repository
isolation checks, Rust tests, 33 Python fixture tests, and the coverage threshold.
The three real-host tests are ignored in normal checks. The explicit A03 Docker
attempt reports blocked with A01/A02/A03 not run because Docker is absent.
Checks and the blocked result are retained under
`$HOME/artifacts/rustward/a03-ssh-20261111/`.

The copied attempt `20261111T044548Z-f2ae255d` reports local controller success
in `docker-06.stdout`. `docker-10.stderr` shows that Docker refused to connect a
container started with `--network=none` to the SSH network. A03 was not admitted,
and no client key was generated. The later export scan could not read its
marker, so it refused raw fixture copies and retained the stopped controller.
The root case fields therefore remain `not run`; they were not populated from
copied case reports. This is a runner failure, not an SSH workload result.

SSH mode now selects its internal network before controller startup. Regression
tests require network creation before startup and reject a later network-connect
step. A failed scan with no JSON report now records a clear failed export
decision instead of a JSON parse error. It still refuses raw copies. The original
copied evidence is unchanged. Diagnostic notes and checks for the fix are under
`$HOME/artifacts/rustward/a03-network-20261111/`.

The copied attempt `20261111T142230Z-4ac2d18b` passed network setup. It failed in
A01 before SSH preparation. A user-supplied decision summary shows Completed with
Interruption, exit code 0, confirmed cleanup, synchronized output, no output
failure, and required review. The first-result rule preserves an earlier
uncertainty even when later exit and cleanup evidence is complete. The exact
observation that caused it is not in the copied logs. A state-read race is a
hypothesis, not a confirmed diagnosis.

SSH mode now prepares its credential marker before running any workload. A local
failure can therefore pass the normal credential scan and export its local
fixture. It does not try to copy an SSH fixture that was never started. Local
command failure records a primary reason in the root result. The Rust example
also prints its structured failure decision, without task output or credential
contents. No task is retried, and no retained result is changed. Checks and
notes are under `$HOME/artifacts/rustward/a01-interruption-20261111/`.

On 2026-11-11, the user accepted up to three fresh read pairs per systemd
observation when state changes but execution identity remains fixed. Each pair
uses a fresh cgroup read; only matching snapshots supply evidence. The first
identity is retained across pairs, including when no stored identity was supplied.
Changed identity, missing units, read errors, or three changed pairs do not
become valid evidence. They retain the existing uncertainty and review behavior.
There is no sleep, new start, or workload retry between read pairs.

A synthetic test reproduces Running changing to Exited between reads. The old
adapter reports uncertainty; the new adapter can obtain stable exit-code-0
evidence from a later pair. Other tests check bounded failure, identity changes
within and across pairs, read errors, and use of the matching pair's cgroup data
for cleanup. This fixes a reproducible race consistent with the A01 report; it
does not establish the exact observation in that earlier run. Existing admitted
runs, results, review blocks, and identities are not rewritten.

The copied attempt `20261111T154745Z-3cc42b56` confirms the local suite passing.
Its A03 stdout records `ControlPath too long` during the copy task. Ansible exits
with code 4; Rustward correctly records Failure with complete output and cleanup,
without a review block. Ansible's SSH plugin detects the presence of
`ControlPersist=no` as a persistence setting and adds its default ControlPath
unless a path is supplied. That default uses the run directory as HOME and exceeds
the Unix socket path limit. An explicit `ControlPath=none` prevents generation
without enabling connection sharing. Existing state and identities are unchanged.

A direct smoke check with the actual Ansible 2.19.0 SSH plugin reproduces the old
policy failure and passes with the fixed SSH and SFTP commands. OpenSSH `-G`
confirms disabled control sockets. Normal tests also check the effective policy
with a later, deliberately long ControlPath option. The installed-plugin check
is now required before A03 admission and in its exported pass evidence. These
command checks do not connect to a target or pass A03. Evidence is under
`$HOME/artifacts/rustward/ssh-control-path-20261111/`.

## Remaining work

Rerun A03 and review its full evidence bundle. A04–A10, production secret
configuration, remote stopping and recovery evidence, runtime selection changes,
output flooding, and storage-failure acceptance remain open. The service loop,
scheduler, reload, retention, UI, migration, and deployment remain outside this
implementation. A03 does not prove production workload compatibility.

Before removal, transfer lasting fixture interfaces to ARCHITECTURE.md, preserve
acceptance evidence, transfer open work, and update inbound links. Follow the
[temporary document lifecycle](README.md).
