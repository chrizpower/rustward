# Rustward

Rustward is a small Rust task executor for one trusted administrator. The local
service runs generic commands through a dedicated systemd user manager.
It supports manual admission, UTC scheduling, live configuration reload,
cancellation, recovery, review, and bounded output capture. Ansible is the
first workload. A private web console supports task submission, run output,
schedule controls, and review. Rustward expires eligible history by age and
supports offline application-state backup and restore, structured service logs,
and private monitoring routes.

Validate the synthetic configuration:

```sh
cargo run --locked -- validate examples/rustward.toml
```

The loader reads version 2 TOML service settings and one file per task and
schedule. Relative paths under the definition roots supply IDs. The service
loads the complete set at startup and supports explicit reload. See the [synthetic settings](examples/rustward.toml).

## Documentation

| Document | Purpose |
| --- | --- |
| [Architecture](docs/ARCHITECTURE.md) | System structure, responsibilities, and execution boundaries |
| [Gravity](docs/GRAVITY.md) | Long-term direction, accepted constraints, and decisions |
| [Glossary](docs/GLOSSARY.md) | Shared domain definitions |
| [Agent instructions](AGENTS.md) | Repository work and documentation rules |

## Download Linux executables

Download the archive and matching `.sha256` file for your architecture from
[GitHub Releases](https://github.com/chrizpower/rustward/releases). Downloads are public.
Set `arch` to `x86_64` or `aarch64` and run these commands in the download directory:

```sh
arch=aarch64
sha256sum --check "rustward-linux-$arch.tar.gz.sha256"
tar -xzf "rustward-linux-$arch.tar.gz"
```

Each archive contains `rustward`, `rustward-exec`, `LICENSE`, and `BUILD-INFO.json`.
Both executables report the packaged version with `--version`.

The [Linux build workflow](.github/workflows/linux-builds.yml) checks the project
and browser console and builds both Linux architectures on the Debian 13 baseline.
After a successful build on `main`, it verifies both archives against their
checksums and source revision, tags the commit, and publishes a GitHub prerelease.
Pull requests run the checks and builds. Earlier tags can have Actions artifacts only.

Build tags use the package version and full commit ID, for example
`v0.1.0-dev.g<full-commit-sha>`. Local builds report a `-dev.local` version.

## Development

Install stable Rust, rustfmt, Clippy, just, Python 3.11+, and build tools.
Coverage checks also need `llvm-tools-preview` (selected in
`rust-toolchain.toml`), and
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov).

Install the coverage tool and enable the repository Git hooks:

```sh
cargo install cargo-llvm-cov --version 0.9.1 --locked
just hooks-install
```

```sh
just check
```

The prepared-runtime tests check directory ownership and permissions. If the
checkout path fails those checks, set `RUSTWARD_TEST_FIXTURE_ROOT` to an
existing absolute directory owned by your user. The directory and its parents
must allow traversal by all users and deny group and other write access.
The parents must belong to your user or root. The path must be free of symlinks.

Rustward runs tasks on Linux with systemd on x86-64 and ARM64. On macOS,
build and test the shared code with Rust and Apple’s Command Line Tools:

```sh
cargo build --locked --all-targets --all-features
cargo test --locked --all-targets --all-features
```

The macOS build includes `rustward validate`. Linux service, task execution,
and host acceptance tests require Linux.

Linux builds also require Node.js 20.19+ in the 20.x series, or 22.12+, and npm.
Cargo installs the locked frontend dependencies and embeds the Vite production
build. The Linux service runs with Rust alone. Native macOS Cargo builds do
not build the console.

Start the synthetic web console:

```sh
just web-preview
```

On Linux this builds and runs the preview directly. On macOS start Docker
Desktop first. The command builds the Linux preview in a container and
publishes it at `http://127.0.0.1:4173`. To use another port, run
`just web-preview-container 4174`. Press Ctrl+C to stop the preview.

For Vite development, start the synthetic preview on port 4173. On Linux run
`cargo run --locked --features web-preview -- web-preview 4173`. On macOS run
`just web-preview-container 4173`. In a second terminal run:

```sh
npm --prefix web ci --include=dev
npm --prefix web run dev
```

Open `http://127.0.0.1:5173`. Vite updates the UI as you edit it and forwards
API calls to the synthetic preview on loopback.

Run the browser tests on Linux:

```sh
just web-test
```

Playwright tests the production assets embedded in the Rust binary. It uses
rootless Podman when available. Run `RUSTWARD_WEB_CONTAINER=none just web-test`
to use local Chromium libraries. Browser failure screenshots and traces go
under `~/artifacts/rustward/web/`. `just check` also runs the frontend source
and TypeScript checks. To run those checks alone, use `just web-check`.

Run the synthetic retention and restore tests on Linux:

```sh
just recovery-test
```

Run `just telemetry-test` on Linux for monitoring checks. To rehearse alert
delivery in an isolated container, run:

```sh
just pilot-test
```

The rehearsal uses Podman or Docker and sends test mail to a loopback SMTP
sink. The full Docker suite also rehearses a backup and restore with systemd.

Run the integration suite from a Linux checkout:

```sh
just acceptance-container
```

This command builds the current source and runs A01–A10, including the local
systemd and SSH cases, in an unprivileged `systemd-nspawn` container. The
Debian 13 runner needs cgroup v2, active BPF LSM, and the systemd mount and
namespace helpers.
Set `RUSTWARD_NSPAWN_BASE_IMAGE` to the runner's prepared GPT image. A Linux
checkout without a local runner can set `RUSTWARD_NSPAWN_RUNNER` to an
executable that forwards input and output to `acceptance/nspawn-runner.sh`.
See the [runner setup](#prepare-an-integration-runner).

To use a prepared disposable QEMU runner on x86-64 Linux, install
`qemu-system-x86` and give the current user access to `/dev/kvm`. Set the path
to the persistent runner directory and run the same command:

```sh
export RUSTWARD_QEMU_STATE_DIR=/absolute/path/to/runner
just acceptance-container
```

The same setting works with the pre-push hook. The launcher uses KVM and
shuts down the VM after the test. The directory must contain the writable
Debian runner disk `runner.qcow2`, its cloud-init seed `seed.iso`, the
synthetic SSH identity `access-key`, and the pinned guest key in
`known-hosts` under the alias `rustward-qemu`. It forwards only local
port 22286 to SSH. The guest uses the `nspawn-runner` account and the
read-only base image `/var/lib/rustward-runner/debian-13-base.raw`.
The launcher sends the current nspawn runner script to the guest.

Run `just acceptance-docker` for the local cases on Docker. Run
`just acceptance-ssh` for all cases on Docker.

No fleet access or private credentials are needed.

`just coverage` runs all test targets and checks Rust line coverage. It excludes
integration-test and example source files from the measurement. Coverage must
be at least 95%. Coverage can decrease while it remains at or above 95%.
`cargo-llvm-cov --fail-under-lines 95` enforces this fixed minimum directly.

The hooks divide the checks as follows:

| Hook | Checks | Manual command |
| --- | --- | --- |
| Pre-commit | Staged file size and whitespace errors, rustfmt, and Clippy with warnings treated as errors | `just check-file-size`, `git diff --cached --check`, and `just check-commit` |
| Pre-push | Rust checks, the 95% line coverage minimum, and A01–A10 acceptance | `just check && just acceptance-nspawn` |

Rust checks use the working tree. For a partial commit, run them with only the
intended changes present. Before a push, run them on the commit to be pushed.
Each checkout needs `just hooks-install` to set the local Git `core.hooksPath`
to `.githooks`.

### Prepare an integration runner

Use a dedicated Linux runner with Debian 13 and systemd 257. Install
`systemd-container` and make the `systemd-mountfsd.socket` and
`systemd-nsresourced.socket` units available. The host kernel must have active
BPF LSM and cgroup v2. Enable linger for a dedicated runner account. Run the
acceptance command as that account.

Prepare a Debian 13 root filesystem with systemd as PID 1. Install `dbus`,
`dbus-user-session`, `libpam-systemd`, `ansible-core`, `openssh-client`, `openssh-server`,
`python3`, `python3-prometheus-client`, `python3-aiosmtpd`, `nginx`, `openssl`,
`prometheus`, `prometheus-alertmanager`, `mount`, and `util-linux`. Create the
non-root `rustward`
account and enable linger. For that account's `user@UID.service`, set
`NoNewPrivileges=yes` and clear `CapabilityBoundingSet` and
`AmbientCapabilities`. Mask the default SSH, Nginx, Prometheus, and
Alertmanager services and the SSH socket. The suite starts its own instances
in the private network namespace. Its SSH targets use separate PID and mount
namespaces and listen only on loopback addresses.
The user manager needs the cgroup v2 memory, PID, and CPU controllers.
The systemd tests read the limits from the run's cgroup.

Put the filesystem in a GPT image with a Linux root partition for the runner's
architecture. Keep this base image read-only to the runner account. Create a
separate ordinary file at `/var/lib/machines/rwtest.raw` and an adjacent lock
file named `.#rwtest.raw.lck`. Give the runner account write access to those
two files and traversal access to the directory. The runner overwrites this
image slot from the base before each test. Keep the machine directory owned by
the administrator. On this dedicated runner, set
`SYSTEMD_MOUNTFSD_TRUSTED_DIRECTORIES=1` in a service drop-in for
`systemd-mountfsd.service` to allow the staged image. The same constraints
apply to an alternate slot selected with `RUSTWARD_NSPAWN_IMAGE_SLOT`.

The Rust build requires the development tools above and `strip` from binutils.
Keep the build distribution compatible with Debian 13. The hook fails if the
runner is unavailable or the test does not produce complete pass evidence.
Logs and evidence go under `~/artifacts/rustward/nspawn/`.

Dependabot groups weekly version updates into one pull request each for Cargo,
npm, GitHub Actions, and Docker when updates are available. Each group includes
major, minor, and patch updates. Debian major image updates are excluded.
Update pull requests use the same CI checks as other pull requests.

The file size check reads only added, copied, modified, renamed, or type-changed
files in the Git index. Each file must be at most 5,000 tokens, where tokens equal
bytes divided by four (20,000 bytes). Unstaged content does not affect this check.
Deleted files and submodules are skipped.

`Cargo.lock` is excluded through `-token-limit` in `.gitattributes`. For other
machine-generated files, add an exact path with `-token-limit` and stage the rule.
The check uses staged attributes. Authored source, tests, and documentation must
stay within the limit. A failure lists the files and gives instructions to split
them. `just check` tests these rules with a synthetic Git repository. The tests
clear inherited Git repository settings before creating fixtures. A regression
test checks that inherited Git paths do not change the parent repository.

Keep deployment configuration outside this repository. Do not copy fleet credentials
or production inventory into this repository.

Rustward is licensed under the [Apache License, Version 2.0](LICENSE).
