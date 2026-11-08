# Rustward

Rustward is a small Rust task executor for one trusted administrator. It will
run generic commands, support manual requests and UTC cron schedules, and
provide a web UI for inputs, logs, and status. Ansible is the first workload.

Status: version 2 configuration validation, request input validation,
argument generation, durable manual admission, and queued cancellation are
implemented. The binary
validates TOML files. The Rust library validates submitted values, returns
separate arguments, and records queued manual runs. A separate supervision
library can request process starts through a systemd user manager; the task
dispatch loop is not implemented. See the
[configuration schema](docs/temporary/configuration.md) and
[synthetic example](examples/rustward.toml).

```sh
cargo run --locked -- validate examples/rustward.toml
```

The loader reads `rustward.toml`, one file per task, and one file per schedule.
Both definition roots allow folders; relative paths without `.toml` supply IDs.
It rejects symlinks and validates the complete set. The old single-document
format is rejected. See the [file contract](docs/temporary/configuration.md#accepted-file-layout).
Cron accepts numeric values, lists, ranges, and `*`. Steps are rejected, and at
least one day field must be `*`. See the [schedule schema](docs/temporary/configuration.md#schedule-schema).
Loading does not activate settings, reload a service, or execute schedules.

## Documentation

The long-lived documentation is README.md, AGENTS.md, and the uppercase-named
references directly in docs/:

| Document | Purpose |
| --- | --- |
| [Agent instructions](AGENTS.md) | Repository work, documentation, and domain language maintenance rules |
| [Architecture](docs/ARCHITECTURE.md) | System structure and implementation boundaries |
| [Gravity](docs/GRAVITY.md) | Accepted architecture rules, authority, and exceptions |
| [Glossary](docs/GLOSSARY.md) | Shared domain definitions |

[Temporary documentation](docs/temporary/README.md) holds requirements, execution
contracts, configuration and capture references, implementation work, and migration
preparation. Their useful content will be absorbed into ARCHITECTURE.md,
GRAVITY.md, and GLOSSARY.md before those temporary files are removed. Their
current rules and contracts remain authoritative within scope until transferred.

Start with architecture and the temporary execution contracts, then consult the
implementation work list. Accepted rules and contracts take precedence over
unaccepted proposals. Acceptance does not prove implementation or testing.

Configuration uses TOML only. The earlier YAML syntax is no longer accepted.
See [conversion and Rust API changes](docs/temporary/configuration.md#format-change-and-compatibility).
Existing admission records and queued runs remain valid.

The library exposes `config::load(path)` and `Config::arguments(task_id,
&values)`. Values use `BTreeMap<String, toml::Value>`. See the [request
contract](docs/temporary/configuration.md#request-inputs-and-arguments) for details.

The `admission::Store` library issues manual request keys and atomically records
queued runs in a dedicated local SQLite database. It can read retained run state
and cancel a queued run without deleting its history or admission record. See the [admission interface
and storage contract](docs/ARCHITECTURE.md#manual-admission-library). There is no
submission CLI, HTTP service, or dispatch implementation.

The `supervision::Supervisor` interface supports replaceable adapters. The
`systemd::Systemd` adapter targets Debian 13, systemd 257, and a dedicated user
manager. Tests use a fake manager and a private D-Bus peer without starting
programs. Runtime verification and durable dispatch remain integration gates. See the [supervision contract](docs/ARCHITECTURE.md#supervision-interface-and-systemd-adapter).

The `capture` library supplies stdout/stderr descriptors and retains separate
raw-byte logs under one configurable per-run limit, defaulting to 10 MiB.
It continues draining after truncation and reports storage failures separately.
See the [capture contract](docs/temporary/capture.md). Aggregate reservations,
retention, and service-wide storage-failure handling remain unimplemented.

Structured service logs and a private Prometheus-compatible `/metrics` endpoint
for the monitoring service are accepted future work. See the [telemetry contract](docs/temporary/telemetry.md).
Task output capture does not implement service telemetry.

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
them. `just check` tests these rules with a synthetic Git repository.

Deployment configuration stays in
[the deployment repository](deployment-repository). Do not copy fleet credentials
or production inventory into this repository. A rootless development container
is not proof that host systemd integration works.

The project license is not selected. Select it before publishing releases.
