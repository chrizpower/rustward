# Temporary documentation

Use these references for active development, verification, and migration.
[Agent instructions](../../AGENTS.md#documentation) define the retention rules.
The [project overview](../../README.md#documentation) lists permanent documents.

| Reference | Status, scope, and removal condition |
| --- | --- |
| [Requirements](requirements.md) | Mixed. Requirement IDs, acceptance criteria, and proposed mechanisms. Remove after the accepted direction is in Gravity and remaining work has an owner. |
| [Configuration](configuration.md) | Implemented. Current schema, loader, and compatibility. Remove when maintained examples, validation, and tests cover the necessary details and architecture retains the ownership model. |
| [Local service](service.md) | Implemented. Service commands, controller, environment, and capture. Remove after integration work is closed or transferred, the supported interface has a maintained source, and lasting boundaries are in architecture. |
| [Durable execution](execution.md) | Implemented. Dispatch, stopping, recovery, and log accounting. Remove after open work is transferred and code and tests preserve the interface and failure cases. |
| [Execution internals](execution-internals.md) | Implemented. Admission, review, and systemd internals. Remove after compatibility and host verification have maintained evidence and remaining work is closed or transferred. |
| [Prepared runtimes](runtime.md) | Implemented. Manifest schemas, verification, and writable directories. Remove after its schema and checks have a maintained source and retirement work is resolved or transferred. |
| [Operational contracts](operations.md) | Implemented scheduling, reload, web access, retention, and restore. Accepted future work for telemetry. Retire sections as work closes or transfers. Keep lasting policy in Gravity. |
| [Acceptance suite](acceptance.md) | Implemented runner. Isolated test setup, commands, and case criteria. Remove once the runnable suite carries its required setup and checks and any open verification has an owner. |
| [Acceptance evidence](acceptance-evidence.md) | Historical verification. Recorded bundle reviews and their limits. Remove after the required evidence is retained with its provenance and any open verification is resolved or transferred. |
| [Migration](migration.md) | Open. Semaphore migration preparation. Remove after migration acceptance or cancellation and transfer of applicable records. |

Remove this index when its last reference is retired. Keep accepted rules and
contracts in force during every transfer.
