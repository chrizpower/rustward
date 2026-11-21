# Rustward

Rustward is a small Rust task executor for one trusted administrator. The local
service runs generic commands through a dedicated systemd user manager.
It supports manual admission, UTC scheduling, live configuration reload,
cancellation, recovery, review, and bounded output capture. Ansible is the
first workload. A private web console supports task submission, run output,
schedule controls, and review. Age-based retention, restore, and telemetry
remain open.

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

## Development

Use the Rustward devbox on the development host. Its checkout is `the checkout`.
The image supplies stable Rust, rustfmt, Clippy, just, Python 3.11+, and build tools.
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

Run the Docker integration suite from the checkout on your laptop with Docker
Desktop running in Linux container mode, or on an isolated Linux Docker host:

```sh
just acceptance-container
```

The Docker suite requires rootful Docker and cgroup v2. It runs synthetic host
tests and checks proxy authentication and a systemd run. It runs separately
from `just check` and `just web-test`.

No fleet access or private credentials are needed. The devbox uses
`the build directory` for compiled output. Source, home,
and build storage persist through container replacement; processes do not.

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
Each checkout needs `just hooks-install`; it sets the local Git `core.hooksPath`
to `.githooks`. GitHub Actions workflows are currently disabled.

The file size check reads only added, copied, modified, renamed, or type-changed
files in the Git index. Each file must be at most 5,000 tokens, where tokens equal
bytes divided by four (20,000 bytes). Unstaged content does not affect this check.
Deleted files and submodules are skipped. Files named `AI.md` are not inspected,
as required by the repository rules.

`Cargo.lock` is excluded through `-token-limit` in `.gitattributes`. For other
machine-generated files, add an exact path with `-token-limit` and stage the rule.
The check uses staged attributes. Authored source, tests, and documentation must
stay within the limit. A failure lists the files and gives instructions to split
them. `just check` tests these rules with a synthetic Git repository. The tests
clear inherited Git repository settings before creating fixtures. A regression
test checks that inherited Git paths do not change the parent repository.

Deployment configuration stays in
[the deployment repository](deployment-repository). Do not copy fleet credentials
or production inventory into this repository.

The project license is not selected. Select it before publishing releases.
