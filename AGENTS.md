# Agent instructions

Read README.md, docs/implementation.md, and docs/plans/executor.md before work.
Use ASD-STE100 Simplified Technical English for prose and new names. Preserve
required interfaces and established technical terms.

Keep the executor generic. Use established Linux infrastructure where it
reduces custom code. Do not add Ansible-specific behavior to the execution core.
Keep production deployment in the deployment repository. Implementation does not
authorize live tasks, schedule migration, or production deployment.

Use synthetic inputs and credentials. Never read, write, search, or ingest
AI.md. Do not copy secrets into code, tests, logs, or documentation.

Keep source and useful artifacts on persistent storage. Put worktrees under
$HOME/worktrees/rustward and reports under $HOME/artifacts/rustward. Use /tmp
only for disposable scratch. Do not prune missing worktrees or run Git garbage
collection during recovery. Active processes do not survive replacement.

Run just check before completion. Add focused tests for behavior and failure
paths as implementation grows. Report failed and skipped checks. Commit the
Cargo lockfile. Do not treat placeholder checks as evidence of executor behavior.

Keep changes within the task. Commit and push only when requested. Stage only
task files. Use feat/, fix/, docs/, refactor/, test/, or chore/ branch prefixes.
