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

```sh
just check
```

The same checks run in GitHub Actions. No fleet access or private credentials
are needed. The devbox uses `the build directory` for compiled output. Source, home,
and build storage persist through container replacement; processes do not.

Deployment configuration stays in
[the deployment repository](deployment-repository). Do not copy fleet credentials
or production inventory into this repository. A rootless development container
is not proof that host systemd integration works.

The project license is not selected. Select it before publishing releases.
