# Rustward

Rustward is a small Rust task executor for one trusted administrator. It will
run generic commands, support manual requests and UTC cron schedules, and
provide a web UI for inputs, logs, and status. Ansible is the first workload.

Status: task configuration validation is implemented. The binary can validate
version 1 YAML files; it cannot run tasks. See the
[configuration schema](docs/configuration.md) and synthetic example.

```sh
cargo run --locked -- validate examples/tasks.yaml
```

Start with [the implementation handoff](docs/implementation.md), then read
[the reviewed plan](docs/plans/executor.md). Confirm phase 0 decisions before
building the execution core. Proposed choices are not measured results.
See [the architecture overview](docs/ARCHITECTURE.md) for the system diagram and
implementation boundaries.

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
