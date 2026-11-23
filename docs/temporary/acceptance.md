# Isolated-host acceptance

Use this suite to verify the generic execution path with a synthetic Ansible
workload and an ordinary command. Run on an isolated Linux host. Results apply
to the recorded fixture and source revision. See [acceptance evidence](acceptance-evidence.md)
for recorded results and limits.

## Docker command and isolation

Run on an isolated Linux Docker host, preferably a disposable VM:

```sh
just acceptance-container
```

The build runs the synthetic retention and restore tests before host acceptance.
The host needs Python 3, rootful Docker, and cgroup v2. Building needs container registry,
Cargo, npm, and Debian repository access. No credentials or production inventory are
required. Docker Desktop requires the same cgroup checks. The runner requires rootful Docker.

The test container is **privileged** so systemd can manage its cgroup hierarchy.
It uses a private cgroup namespace, systemd as PID 1, and a dedicated UID 1000
user manager. A privileged container is not a security boundary from its Docker
host. Use a disposable host, never a production daemon. See [Docker privileges](https://docs.docker.com/reference/cli/docker/container/run/#escalate-container-privileges---privileged)
and [systemd container requirements](https://systemd.io/CONTAINER_INTERFACE/).

The runner supplies no host mounts or host sockets. Local mode has no network.
Remote mode creates an internal network before controller startup and joins
targets after the local cases. The dedicated manager and executor use restricted
privileges. The build context contains Cargo and Vite build inputs, source, fixtures, the
acceptance example, and required host-test sources. Source symlinks are rejected.

Run A01–A10 with disposable SSH targets:

```sh
just acceptance-ssh
```

Rust, Node.js, and Debian image references are pinned by digest. Optional explicit image
references can be supplied for a controlled update:

```sh
python3 -I -B acceptance/container.py --rust-image '<reference>' --debian-image '<reference>'
```

The build records Debian package versions because rebuilds can select different
revisions. The runner starts the fixed image ID with pulling disabled. Source
hashes identify the exact build context, including uncommitted changes.

## Preparation and dependencies

Fixture preparation happens as root before controller execution. It requires a
new absolute root. Existing roots are never replaced. It copies a Python
interpreter, its standard library, installed Python packages, and required
ordinary utilities into a symlink-free runtime. Ansible builtin collection code,
playbook, inventory, wrapper, and configuration are included. No collection is
fetched during a run.

The fixture uses `ldd` only on trusted project binaries and distribution ELF
files during preparation. It pins the launcher, shell, ELF dependencies, explicit
host symlink targets, loader cache, locale data, and relevant non-secret account
configuration in a host manifest. Runtime and host IDs are computed by the
production Rust parsers through the example's `id` command, not a second hash
implementation. Catalogs and immutable fixture files belong to root. The
controller checks the execution user, manager, paths, hashes, and privileges.

Preparation owns dependency completeness. Fix missing dependencies in the fixture
before running it again.

## Portable host command

The same Rust harness and preparation can run in a Debian 13 VM without Docker.
Provision systemd 257, cgroup v2, the Dockerfile's packages, and a dedicated
non-root user through the VM image or configuration management. Apply the
Dockerfile's privilege settings to that dedicated user-manager unit as well. Do not reuse an
administrator's ordinary user manager. The runner starts and stops the dedicated
manager, including after test failure.

Build and place these five executables in a protected tools directory:

- `cargo build --locked --release --bin rustward --bin rustward-exec --example acceptance`
- `cargo test --locked --release --no-run --test systemd_host --test runtime_host`
- Copy the example, service, launcher, and test executables into the tools
  folder as `acceptance`, `rustward`, `rustward-exec`, `systemd_host`, and
  `runtime_host`.

Use a new fixture root on persistent storage, with protected ancestors. Then run
this explicit root setup command from the checkout:

```sh
just acceptance-host /opt/rustward-acceptance /opt/rustward-acceptance-tools rustward
```

The recipe records HEAD as its source-revision label. Preserve the source diff
and build inputs separately if this VM build includes uncommitted changes. The
Docker runner records a complete content-hash snapshot automatically.

For prepared fixtures, `acceptance run <root>` must run as the fixture execution
user with its dedicated XDG_RUNTIME_DIR and DBUS_SESSION_BUS_ADDRESS. The example
refuses another UID. `acceptance probe-manager` only authenticates and reads the
manager. It does not open run storage or start a task. The `recovery-child`
command is an internal test mechanism.
These commands belong to the acceptance example.

## Evidence, failure, and cleanup

The Docker command writes a unique directory under
`$HOME/artifacts/rustward/ansible-acceptance/`. Override it with an unused absolute
`--artifacts` path. It records commands as argument arrays, output, exit status,
source hashes, fixed image ID, Docker inspection, journal data, fixture versions,
runtime/host manifests, task configuration, per-case decisions, database files,
work markers, and captured streams. The local fixture is copied before container removal, including on failure.
In remote mode, a credential scan must pass before either fixture is copied. A
failed scan retains the stopped controller and refuses raw export.

The host runner ends test processes and the dedicated user manager before
returning. Database files are copied after writers have stopped, not while a
live controller is writing them. Case reports preserve state, result, launch
identity, cleanup, output accounting, and observed failure. They do not rewrite
history during cleanup.

A zero exit code without the required collected case evidence does not pass.
Blocked, failed, incomplete, and not-run cases stay distinct from passes. Failed
Docker diagnostics still lead to an attempt to collect data and stop the
container. If evidence copy, diagnostics, or cleanup cannot finish, the runner
returns failure and records the retained container name. It can still be running
if stopping failed. Inspect it and preserve missing evidence before removal.
There is no global prune, volume removal, or unrelated container cleanup.

Preparation and program execution use deadlines. The recovery kill waits on
recorded Running state and explicit markers. It does not use a sleep duration as
proof that a program started. Manager calls and filesystem I/O still have their
normal failure limits. A timeout is a failure, not cleanup evidence.

## Local checks

- A01 runs a local Ansible playbook. The workload wrapper turns separate string,
  integer, and boolean arguments into JSON extra variables. It passes a string
  with spaces and both quote types, integer 7, and boolean true. The playbook
  checks their types, writes known JSON in the writable run directory, reads it,
  and checks it again. See [Ansible variable types](https://docs.ansible.com/projects/ansible/latest/user_guide/playbooks_variables.html).
- A02 runs an ordinary Python command through the same path. It checks the exact
  seven-variable environment, non-root UID, zero held capabilities, NoNewPrivileges,
  and input values, writes known output, and starts
  a detached child. The main process exits with code 0 while the child remains.
  Success requires controller cleanup and proof that both processes ended.
- An additional **local-recovery** check kills a separate controller process with
  SIGKILL after durable Running evidence and an explicit workload marker. It
  reopens the controller, checks retained invocation/cgroup identity, proves no
  second workload start, completes local cleanup with Interruption and review,
  and checks that another admitted run stays queued. This does not pass A06,
  which requires remote-work evidence.

A01/A02 require Completed/Success, exit code 0, retained execution identity,
confirmed cleanup, no review or capture failure, synchronized non-recovered
output, matching byte accounting, and both stdout/stderr markers. Completion
must survive database reopen. Start markers prove one workload invocation.
Process markers include PID and Linux start time to handle PID reuse.

The recovery path reacquires one systemd unit reference before stopping an
existing execution. RefUnit never starts a missing unit. The adapter checks
identity before and after acquisition, then retains the unit until cleanup and
release. See the [systemd reference interface](https://www.freedesktop.org/software/systemd/man/257/org.freedesktop.systemd1.html).

Before the suite, the host runner waits up to 30 seconds for a read-only
`acceptance probe-manager` response. It uses the native UserBus authentication
path, requires the execution UID and systemd 257, and records every attempt in
`fixture/evidence/manager-readiness.json`. A listening bus socket alone does not
prove that the manager has registered its D-Bus name. Only readiness probes are
repeated. Host tests and task starts are not retried. The controller's normal
fail-closed behavior is unchanged.

The host runner requires three opt-in passes for child cleanup and unit
isolation, output capture, and runtime permissions. The current runner
also requires daemon, scheduler, and HTTPS proxy passes. Their reports are
`fixture/evidence/daemon.json`, `scheduler.json`, and `web.json`. See
[private web access](operations.md#private-web-access) for browser tests.
Install Nginx and OpenSSL for the web case on a prepared VM.

For a direct systemd check, run `cargo test --locked --test systemd_host -- --ignored`
on the isolated host. The
test `prepared_runtime_host_permissions` in `runtime_host` uses a separate
protected fixture. Set `RUSTWARD_PREPARATION_UID`, `RUSTWARD_RUNTIME_CATALOG`,
`RUSTWARD_RUNTIME_ID`, `RUSTWARD_RUNTIME_PROGRAM`, `RUSTWARD_RUNTIME_CWD`,
and `RUSTWARD_RUNTIME_WORK_ROOT`, then run as the non-root execution user:

```sh
cargo test --locked --test runtime_host -- --ignored --exact prepared_runtime_host_permissions
```

## Credential and host-key rules

Root setup generates a new Ed25519 client key inside the controller container
before the local suite starts, so its export marker is available if A01 fails.
The key stays at `/run/rustward-credentials/client_key`, outside exported
fixture roots. Only the execution UID can read it (mode 0400). The enclosing
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
Only readiness probes can repeat. An admitted task is never retried. The wrong
key check does not pass A04's unreachable-workload case.

The separate target runs ordinary sshd on port 2222, with public-key access only
for a synthetic non-root account. It disables password login, root login,
forwarding, and interactive terminals. Python is installed before execution.
The controller runtime copies SSH and SFTP binaries and pins their ELF host
dependencies through the existing manifest preparation path.

## Evidence and failure handling

After all writers stop, scan the local, SSH, stopping, and A06–A10 fixture
roots, including A10's exported copy. The scan checks file names and data for a
generated marker of at least 64 bytes, including matches across read boundaries.
It refuses links, special files, and excluded entries. Its detection scope is the
literal marker. Every raw export requires a pass. On a scan failure retain the
stopped controller and report affected paths with marker-bearing names hidden.

Record fixed target image and package metadata, public keys, the scan decision,
and independent target observations. For A03, retain `target-check.json` and
`ssh-fixture/evidence/A03.json` with its command-builder checks. Keep private keys
outside exported fixture roots. Cleanup targets and networks by name.

## Case criteria

For each case, record the fixture revision, runtime identity, submitted
non-secret inputs, run IDs, trigger, expected result, observed result, and
evidence paths. Inspect both Rustward state and the test target where relevant.

| ID | Action and conditions | Pass criteria |
| --- | --- | --- |
| A01 | Submit the local playbook through manual admission. Repeat the same submission key and payload | Both submissions identify one run. The playbook starts once, receives the expected typed values, writes the expected content, and completes with Success after local cleanup and output capture |
| A02 | Run an ordinary command through the same admission and execution path | It completes with the expected result and output. No Ansible-specific branch or dependency is added to the execution core |
| A03 | Run the playbook against the isolated SSH target using synthetic credentials | The expected target change is verified. Host-key verification remains enabled, no interactive input is needed, and the synthetic secret marker is absent from retained service data and output |
| A04 | Cause a playbook task to fail. Separately make the target unreachable | Each run records Failure with exit evidence and useful output. Success is not reported. Local cleanup is confirmed before its execution slot is released |
| A05 | Cancel a queued run. Separately cancel and time out active remote work at a known synchronization point | The queued run never starts. Active runs retain Cancellation or Timeout as applicable. All local children end before slot release. Uncertain external work blocks further dispatch pending review |
| A06 | Kill the Rustward service after remote work starts, with another run queued. Restart it | Recovery preserves queued work, does not automatically retry interrupted work, and records Interruption where the normal result cannot be established. Uncertain external work blocks new starts. Acknowledgement requires local completion and review of current evidence |
| A07 | Start remote asynchronous work and interrupt its local controller while the remote action remains observable | Evidence distinguishes local cleanup from remote completion. Dispatch remains blocked while external work is uncertain. Review records what happened remotely or explicitly accepts remaining risk. It does not turn the interrupted result into Success |
| A08 | Admit work with runtime A, then select runtime B before dispatch. Repeat while A is active | Queued and active runs retain A and its fixed code and dependencies. New admissions can select B. Removing a selected runtime causes explicit failure, with no substitution of B |
| A09 | Generate output beyond the configured per-run limit | Retained output stays within the limit, truncation is recorded, output continues to drain, and the playbook does not block on full output pipes |
| A10 | Inject a log-storage failure during active remote work, with another run queued | Admission and dispatch close, local stopping starts without requiring another successful database write, and no unsupported durable success is reported. Uncertain external work requires review under P06 |

A01 and A02 establish the first local integration result. A03 through A10 are
required before this acceptance test is complete. Existing generic tests still
own broader crash-window, retention, concurrency, and storage coverage. These
workload cases do not replace them. UI and scheduled requests need later
integration tests against the same execution path.

### Remote case evidence

- **A04:** The deliberate task failure records an independent target proof and
  Ansible exit 2. Stop the target and confirm `Running=false` and `Pid=0` before
  admitting the unreachable case. That case requires Ansible exit 4 and an
  unreachable report. Both runs retain Failure, synchronized capture, and
  confirmed cleanup. See `ssh-fixture/evidence/A04.json` and its subcase reports.
- **A05:** Use a fresh target and separate host pin. Active cases require a
  matching live marker with run ID, typed inputs, PID, and process start time.
  The timeout uses 60 real seconds from dispatch, with a 45-second startup
  deadline and a two-second cleanup grace. Check both review blocks after
  reopening the separate cancellation and timeout stores. Docker checks that
  the queued case marker is absent before and after. See
  `ssh-fixture/evidence/A05.json` and the two `stopping-*-fixture/` stores.
- **A06 and A07:** Kill the child controller after durable Running and a
  matching remote marker. Check one start, retained invocation/cgroup identity,
  queued work, and unknown historical output counts. For A07 independently
  confirm the same remote async process is still alive after local cleanup.
  Acknowledge one completed run with the unchanged evidence. Confirm its
  result stays Interruption and the queued run stays queued.
- **A08:** Use separate prepared A and B runtime trees. Check the original
  snapshot, arguments, revisions, and fingerprint while A is queued, active,
  and complete. A third admission selects A before removal. Its failure must
  record no start and preserve its selection. Keep the displaced runtime under
  `removed-runtimes/` for evidence.
- **A09:** Emit 1 MiB on each stream remotely and require independent target
  completion evidence. The local capture retains exactly 65,536 bytes and
  records truncation and Success. Either stream can consume the allowance.
- **A10:** Exhaust a private 1 MiB log tmpfs during a held SQLite write lock.
  Confirm the local unit stops and the gate closes before unlock, without a
  saved result. After unlock, require Interruption, failed output, review,
  and queued work without a start. Confirm zero free bytes on the private
  log filesystem. After stopping all writers, save both streams to
  `exported-logs/` and compare lengths and SHA-256 hashes with
  `evidence/A10-export.json`. A missing stream prevents a pass even when
  its expected length is zero.

A06–A10 use `remaining-A06-fixture/` through `remaining-A10-fixture/` with
their own stores, decisions, and cleanup reports. Independent Docker
observations use `remaining-A06-target.json` through `remaining-A10-target.json`.
Check all created stores for stopped writers before raw export. Keep prior
decisions and review blocks intact.
