# A03 SSH acceptance

Status: Implemented, not run. The user accepted fixture-only credential files
on 2026-11-11. Docker is absent from the devbox. Synthetic tests do not pass A03.
The user reported the preceding A01/A02 suite passing on Docker Desktop; the
complete successful bundle has not been independently inspected here.

## Run and scope

Run on the same isolated Docker host used for the local acceptance suite:

```sh
just acceptance-ssh
```

For an explicit evidence folder:

```sh
python3 -I -B acceptance/container.py --remote --artifacts "$HOME/artifacts/rustward/a03-run"
```

Use a new folder. The command first runs the unchanged A01/A02 and local recovery
suite without a network. It then builds a disposable SSH target and connects both
containers to a unique internal Docker network. No host ports, source mounts,
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

Root setup generates a new Ed25519 client key inside the controller container.
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
inherited client configuration with `-F /dev/null`. See
[OpenSSH configuration](https://man.openbsd.org/ssh_config) and the
[Ansible SSH connection options](https://docs.ansible.com/projects/ansible-core/2.18/collections/ansible/builtin/ssh_connection.html).

Before admission, a read-only SSH probe with a wrong pinned host key must fail
with host-key verification evidence. A probe with the correct key must succeed.
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
isolation checks, Rust tests, 28 Python fixture tests, and the coverage threshold.
The three real-host tests are ignored in normal checks. The explicit A03 Docker
attempt reports blocked with A01/A02/A03 not run because Docker is absent.
Checks and the blocked result are retained under
`$HOME/artifacts/rustward/a03-ssh-20261111/`.

## Remaining work

Run A03 and review its full evidence bundle. A04–A10, production secret
configuration, remote stopping and recovery evidence, runtime selection changes,
output flooding, and storage-failure acceptance remain open. The service loop,
scheduler, reload, retention, UI, migration, and deployment remain outside this
implementation. A03 does not prove production workload compatibility.

Before removal, transfer lasting fixture interfaces to ARCHITECTURE.md, preserve
acceptance evidence, transfer open work, and update inbound links. Follow the
[temporary document lifecycle](README.md).
