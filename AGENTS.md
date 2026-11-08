# Agent instructions

Before work, read [README.md](README.md), [architecture](docs/ARCHITECTURE.md),
and [Gravity](docs/GRAVITY.md). While present, also read
[requirements](docs/temporary/requirements.md),
[execution contracts](docs/temporary/execution.md), and the
[implementation work list](docs/temporary/implementation.md).
Follow accepted rules and contracts within scope; unaccepted proposals remain
proposals. Read [the glossary](docs/GLOSSARY.md) for domain work and follow its
maintenance instructions. Use ASD-STE100 Simplified Technical English for prose
and new names. Preserve required interfaces and established technical terms.

The long-lived documentation is README.md, AGENTS.md, and the uppercase-named
references directly in docs/. Keep supporting documents in docs/temporary/,
including requirements, execution contracts, configuration details, work lists,
and migration plans. Follow the [lifecycle guidance](docs/temporary/README.md).
Before deleting a temporary file, absorb its lasting content into ARCHITECTURE.md,
GRAVITY.md, or GLOSSARY.md, preserve useful evidence, resolve or transfer open
work, and update inbound links. Temporary location does not weaken accepted rules
or implemented contracts.

Update the document that owns the affected behavior or decision. README.md maps
document roles. Keep its entry points and the architecture overview consistent
when behavior, setup, structure, implementation status, or open decisions change.
Distinguish implemented behavior, accepted future contracts, and proposals.

Keep the executor generic. Use established Linux infrastructure where it reduces
custom code. Do not add Ansible-specific behavior to the execution core. Keep
production deployment in the deployment repository. Implementation does not authorize
live tasks, schedule migration, or production deployment.

Use synthetic inputs and credentials. Never read, write, search, or ingest
AI.md. Do not copy secrets into code, tests, logs, or documentation.

Keep source and useful artifacts on persistent storage. Put worktrees under
$HOME/worktrees/rustward and reports under $HOME/artifacts/rustward. Use /tmp
only for disposable scratch. Do not prune missing worktrees or run Git garbage
collection during recovery. Active processes do not survive replacement.

Run just check before completion. Add focused tests for behavior and failure
paths as implementation grows. Report failed and skipped checks. Keep the Cargo
lockfile in version control and include dependency changes when a commit is
requested. Do not treat placeholder checks as evidence of executor behavior.

Keep changes within the task. Commit and push only when requested. Stage only
task files. Use feat/, fix/, docs/, refactor/, test/, or chore/ branch prefixes.
