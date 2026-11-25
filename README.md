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

[Linux builds](.github/workflows/linux-builds.yml)
runs the project checks in the Debian 13 build container and the browser tests
on the GitHub runner. It builds both Linux architectures on pull requests to
`main` and pushes to `main`. Successful pushes upload Actions artifacts
and create a version tag after verifying both archives.

The version starts with the package version in `Cargo.toml`. For example,
an artifact from package version `0.1.0` reports `0.1.0-dev.g<full-commit-sha>`.
Both executables report that value with `--version`. The web console and
`BUILD-INFO.json` show the same version, and the Git tag adds a leading `v`.
The tag message links to the workflow run. Local Cargo builds report a
`-dev.local` version. Change the package version in `Cargo.toml` and update
`Cargo.lock` to select a new base version. Existing `build-<full-commit-sha>` tags
still identify earlier artifacts.

To find the newest completed versioned build on `main`, run from a checkout:

```sh
git fetch origin --tags
tag=$(git describe --first-parent --tags --abbrev=0 --match 'v[0-9]*.g*' origin/main)
printf '%s\n' "${tag#v}"
git tag -ln "$tag"
```

The printed version matches `rustward --version`. The tag message contains the
download run URL. The lookup stays on the last completed build while a newer
commit is still building.

Choose the archive for your Linux server's architecture. Each archive contains
`rustward`, `rustward-exec`, `LICENSE`, and `BUILD-INFO.json`. Both executables come from the
same locked build. The build and smoke tests use the Debian 13 baseline.

To download by exact source revision, sign in to GitHub CLI with repository
read access and run the following. Set `arch` to `x86_64` or `aarch64`.

```sh
repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
revision=REPLACE_WITH_FULL_COMMIT_SHA
arch=aarch64
run=$(gh run list --repo "$repo" --workflow linux-builds.yml --commit "$revision" \
  --status success --limit 1 --json databaseId --jq '.[0].databaseId')
gh run download "$run" --repo "$repo" --name "rustward-linux-$arch" --dir downloads
cd downloads
sha256sum --check "rustward-linux-$arch.tar.gz.sha256"
tar -xzf "rustward-linux-$arch.tar.gz"
```

GitHub Actions artifacts expire under the repository's retention policy.
Keep a copy and its checksum with the pinned build inputs for server installations
that must be repeatable after the artifact expires.

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

Run the Docker integration suite from the checkout on your laptop with Docker
Desktop running in Linux container mode, or on an isolated Linux Docker host:

```sh
just acceptance-container
```

The Docker suite requires rootful Docker and cgroup v2. It runs synthetic host
tests and checks proxy authentication and a systemd run. It runs separately
from `just check` and `just web-test`.

No fleet access or private credentials are needed.

`just coverage` runs all test targets and checks Rust line coverage. It excludes
integration-test and example source files from the measurement. Coverage must
be at least 95%. Coverage can decrease while it remains at or above 95%.
`cargo-llvm-cov --fail-under-lines 95` enforces this fixed minimum directly.

The hooks divide the checks as follows:

| Hook | Checks | Manual command |
| --- | --- | --- |
| Pre-commit | Staged file size and whitespace errors, rustfmt, and Clippy with warnings treated as errors | `just check-file-size`, `git diff --cached --check`, and `just check-commit` |
| Pre-push | rustfmt, Clippy, all test targets, and the 95% line coverage minimum | `just check` |

Rust checks use the working tree. For a partial commit, run them with only the
intended changes present. Before a push, run them on the commit to be pushed.
Each checkout needs `just hooks-install` to set the local Git `core.hooksPath`
to `.githooks`.

Dependabot opens weekly update pull requests for Rust, npm, GitHub Actions,
and pinned Docker images. Update pull requests use the same CI checks as
other pull requests.

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
