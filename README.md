# Rustward

Rustward is a small Rust task executor for one trusted administrator. It will
run generic commands, support manual requests and UTC cron schedules, and
provide a web UI for inputs, logs, and status. Ansible is the first workload.

Status: version 1 configuration validation, request input validation, and
argument generation are implemented. The binary validates YAML files. The Rust
library validates submitted values and returns separate arguments; it cannot run
tasks. See the [configuration schema](docs/temporary/configuration.md) and [synthetic
example](examples/tasks.yaml).

```sh
cargo run --locked -- validate examples/tasks.yaml
```

## Documentation

The long-lived documentation is README.md, AGENTS.md, and the uppercase-named
references directly in docs/:

| Document | Purpose |
| --- | --- |
| [Agent instructions](AGENTS.md) | Repository work and documentation maintenance rules |
| [Architecture](docs/ARCHITECTURE.md) | System structure and implementation boundaries |
| [Gravity](docs/GRAVITY.md) | Accepted architecture rules, authority, and exceptions |
| [Glossary](docs/GLOSSARY.md) | Shared domain language and maintenance guidance |

[Temporary documentation](docs/temporary/README.md) holds requirements, execution
contracts, the configuration reference, implementation work, and migration
preparation. Their useful content will be absorbed into ARCHITECTURE.md,
GRAVITY.md, and GLOSSARY.md before those temporary files are removed. Their
current rules and contracts remain authoritative within scope until transferred.

Start with architecture and the temporary execution contracts, then consult the
implementation work list. Accepted rules and contracts take precedence over
unaccepted proposals. Acceptance does not prove implementation or testing.

The library exposes `config::validate` and `Config::arguments(task_id,
&values)`. Values use `BTreeMap<String, serde_yaml_ng::Value>`. See the [request
contract](docs/temporary/configuration.md#request-inputs-and-arguments) for details.

## Development

Use the Rustward devbox on the development host. Its checkout is `the checkout`.
The image supplies stable Rust, rustfmt, Clippy, just, and build tools.
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
| Pre-commit | Staged whitespace errors, rustfmt, and Clippy with warnings treated as errors | `git diff --cached --check` and `just check-commit` |
| Pre-push | rustfmt, Clippy, all test targets, and the 95% line coverage minimum | `just check` |

Rust checks use the working tree. For a partial commit, run them with only the
intended changes present. Before a push, run them on the commit to be pushed.
Each checkout needs `just hooks-install`; it sets the local Git `core.hooksPath`
to `.githooks`. GitHub Actions workflows are currently disabled.

Deployment configuration stays in
[the deployment repository](deployment-repository). Do not copy fleet credentials
or production inventory into this repository. A rootless development container
is not proof that host systemd integration works.

The project license is not selected. Select it before publishing releases.
