# Local acceptance runner

Status: The copied bundle confirms A01/A02, local recovery, and all three legacy
host tests.
See the [evidence review](acceptance-review.md) for the latest A01–A10 pass,
including A10's exported output. Docker is absent from the devbox.

Before removal, transfer stable test interfaces and evidence to ARCHITECTURE.md,
policy to GRAVITY.md, and terms to GLOSSARY.md. Preserve the remaining host,
remote-work, and deployment gates, useful evidence, and inbound links.

## Scope

The Rust example `acceptance` uses Store, Controller, pinned runtime/host catalogs,
UserBus, and the systemd adapter. It admits manual work twice with the same key
and payload, checks one run ID, starts the retained run, and polls to completion.
There is no fake supervisor or Ansible branch in the core.

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
repeated; host tests and task starts are not retried. The controller's normal
fail-closed behavior is unchanged.

The runner also executes the three existing opt-in tests: systemd child cleanup
and unit isolation, systemd output capture, and prepared-runtime permissions.
Zero tests executed is a failure. The suite requires their exact pass counts.

The [copied review](acceptance-review.md) confirms A01–A10, including A10's exported logs.
Production credentials and workload compatibility, UI, scheduling, reload,
retention, telemetry, migration, and deployment remain open. A03 adds
fixture-only credential files through a separate opt-in command. The local
suite needs no credentials or conditional arguments.

## Docker command and isolation

Run on an isolated Linux Docker host, preferably a disposable VM:

```sh
just acceptance-container
```

The host needs Python 3, rootful Docker, and cgroup v2. Building needs registry,
Cargo, and Debian repository access. No credentials or production inventory are
required. Docker Desktop's Linux VM can be a candidate; its cgroup support must
pass the same checks. Rootless Docker is rejected by this runner.

The test container is **privileged** so systemd can manage its cgroup hierarchy.
It uses a private cgroup namespace, systemd as PID 1, and a dedicated UID 1000
user manager. A privileged container is not a security boundary from its Docker
host. Use a disposable host, never a production daemon. See [Docker privileges](https://docs.docker.com/reference/cli/docker/container/run/#escalate-container-privileges---privileged)
and [systemd container requirements](https://systemd.io/CONTAINER_INTERFACE/).

The runner does not mount host source, a Docker socket, host cgroups, or a host
D-Bus socket. The local-only command uses no network. A03 mode starts the controller on an
internal network with no target peer during the local suite, then starts its
disposable SSH target on that network. A separate build step prepares
all dependencies before execution. Private writable run/log/state folders belong
to the execution user. The controller and tests run with all Linux capabilities
removed through setpriv; the root setup process is not the executor. The image
also configures its dedicated user manager with NoNewPrivileges and empty
capability bounding and ambient sets before the UID switch.

The build context contains only Cargo inputs, src, this fixture, the acceptance
example, and the two required host-test sources. It excludes Git metadata,
unrelated files, caches, and AI.md. It rejects source symlinks. Git recovery and
production checkout data are not part of the container filesystem.

Rust and Debian image references are pinned by digest. Optional explicit image
references can be supplied for a controlled update:

```sh
python3 -I -B acceptance/container.py --rust-image '<reference>' --debian-image '<reference>'
```

Debian package versions are resolved at image build time, then recorded. Rebuilds
can select different package revisions; the fixed image ID and manifests identify
one actual test selection. This is not a promise of identical image rebuilds.
The runner starts that image ID with pulling disabled and never updates packages
while a run is active. Source content hashes identify the exact build context,
including uncommitted changes, rather than claiming it equals HEAD.

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

Dependency completeness remains a preparation responsibility. A host-specific
library error is a failure to fix, not permission to skip verification or fetch
another dependency during execution. The fixture copies package code; it is not
a general runtime preparation or deployment tool.

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
manager; it does not open run storage or start a task. The `recovery-child`
command is an internal test mechanism.
These commands do not extend the product CLI or create a service loop.

## Evidence, failure, and cleanup

The Docker command writes a unique directory under
`$HOME/artifacts/rustward/ansible-acceptance/`. Override it with an unused absolute
`--artifacts` path. It records commands as argument arrays, output, exit status,
source hashes, fixed image ID, Docker inspection, journal data, fixture versions,
runtime/host manifests, task configuration, per-case decisions, database files,
work markers, and captured streams. The local fixture is copied before container removal, including on failure.
In A03 mode, a credential scan must pass before either fixture is copied; a
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
returns failure and records the retained container name; it can still be running
if stopping failed. Inspect it and preserve missing evidence before removal.
There is no global prune, volume removal, or unrelated container cleanup.

Preparation and program execution use deadlines. The recovery kill waits on
recorded Running state and explicit markers. It does not use a sleep duration as
proof that a program started. Manager calls and filesystem I/O still have their
normal failure limits; a timeout is a failure, not cleanup evidence.

## Verification recorded in the devbox

The devbox cannot run Docker, and normal checks skip the three host tests.
The copied Mac attempt `20261110T005145Z-08053ac4` built and booted its container.
It passed confirms_child_cleanup_and_concurrent_unit_isolation. Its capture test
failed at UserBus::connect with Unavailable before capture setup or a task start.
The later test connected successfully to the same manager. This suggests a
startup readiness race; the fixed error classification does not establish the
exact D-Bus error. The runtime permission test, A01/A02, and local recovery did
not run. This partial result is not a complete acceptance pass.

The runner now waits for authenticated manager readiness before the first host
test. Synthetic tests cover delayed readiness, failed or hung probes, and wrong
UID/version replies. The user reported a later complete local pass for source revision
`523c67bc54608481d156b8a8b21526d780cc16318f81113025427686c56ad268`.
The successful full bundle has not been independently inspected here. Diagnostic copies
are under `$HOME/artifacts/rustward/manager-readiness-20261111/evidence/`.

Normal checks build the Rust example and test evidence validation, duplicate
starts, process identity handling, wrapper argument types, manifest construction,
source isolation, non-root privilege drops, Docker failure reporting, required
pass evidence, and collection before cleanup. They use synthetic data and mocks;
they are not host acceptance.

Separate direct smoke tests ran the actual local playbook and ordinary wrapper
through rustward-exec with a copied interpreter and fixed Python wheels. The
Ansible playbook completed with the expected typed values. Manifests for the
copied dependency set were accepted by the production hash parsers. These smoke
tests did not use Controller or systemd and do not pass A01/A02. Evidence is under
`$HOME/artifacts/rustward/acceptance-20261109/`.
