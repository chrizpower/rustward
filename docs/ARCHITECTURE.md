# Rustward architecture

Rustward will run generic command tasks for one trusted administrator. Version 1
configuration validation, request input validation, and argument generation are
implemented. The diagram shows the
planned system and marks the implemented parts in green.

This overview follows the accepted rules in [docs/GRAVITY.md](GRAVITY.md).
Detailed mechanisms that remain proposals in the
[executor plan](plans/executor.md) remain proposals here. This diagram does
not authorize execution, migration, or deployment.

## System diagram

```mermaid
flowchart TB
    Git["Git-managed YAML task definitions"]
    CLI["rustward validate<br/>Implemented"]
    Config["Versioned configuration validation<br/>Implemented"]

    Admin["Administrator"]
    Proxy["Private HTTPS proxy<br/>Authentication"]

    subgraph Service["Planned Rustward service"]
        UI["Web UI and API"]
        Cron["UTC scheduler<br/>Skip missed runs"]
        Inputs["Input validation and argument mapping<br/>Implemented library"]
        Queue["Durable admission and bounded queue"]
        Dispatch["Dispatch and recovery controls"]
        Capture["Output capture"]
    end

    DB[("Local SQLite<br/>Requests, runs, and state")]
    Prepare["External runtime preparation"]
    Runtime["Fixed code and dependencies"]
    Systemd["systemd task supervision"]
    Task["Generic command task"]
    Logs[("Bounded stdout and stderr logs")]

    Git --> CLI --> Config
    Config --> Inputs

    Admin --> Proxy --> UI
    UI --> Inputs
    Cron --> Inputs
    Inputs --> Queue --> Dispatch
    Queue <--> DB
    Dispatch <--> DB

    Prepare --> Runtime
    Dispatch --> Systemd --> Task
    Runtime --> Task
    Task --> Capture --> Logs
    Logs --> UI
    DB --> UI

    classDef implemented fill:#d9ead3,stroke:#38761d,stroke-width:2px
    class CLI,Config,Inputs implemented
```

The service boundary shows the planned single Rust service. The proxy, systemd,
task processes, and runtime preparation are outside that service. The local
SQLite database and log files store retained state and output. Output capture
is a logical component; its systemd interface remains to be defined.

## Current implementation

`rustward validate <config.yaml>` reads one YAML document and checks its version,
task IDs, typed inputs, defaults, choices, and argument references. It rejects
unknown fields and duplicate IDs. It does not execute commands or check program
availability. See the [configuration schema](configuration.md) and
[synthetic example](../examples/tasks.yaml).

The Rust library exposes `config::validate` and `Config::arguments`. A validated
configuration selects a task by ID, rejects undeclared or invalid values, applies
defaults, and rejects missing required inputs. It returns an ordered `Vec<String>`
with one value per configured argument. It validates unused inputs too. It does
not split, expand, or interpolate values. There is no submission CLI or command
execution. The implemented library is shown inside the planned service boundary;
service integration is future work.

## Accepted boundaries

- **C01:** Use Rust and generic command tasks. Keep Ansible-specific behavior
  outside the execution core. Production deployment belongs in `the deployment repository`.
- **P01:** Use one service, local SQLite, embedded web assets, and Linux systemd
  supervision. Authenticate one administrator at the private proxy and protect
  state-changing browser requests in Rustward.
- **P02:** Preserve queued work and block new starts when external work is
  uncertain. Do not retry interrupted work automatically. For planned
  maintenance, pause new starts and wait for active tasks to finish.
- **P03:** Prepare fixed code and dependencies outside execution. Queued and
  active work must retain its selected runtime.
- **P06:** Close admission and dispatch and stop active local work if state or
  log storage fails. Normal log truncation must continue to drain output.
- **P07:** Enforce configurable bounds. Accepted starting values are one active
  task, 100 queued requests, 10 MiB of logs per run, 1 GiB of retained logs, and
  30-day retention. These limits are not implemented yet.

## Open decisions

Before execution integration, define runtime identity and layout, the systemd
baseline, manager type and unit permissions, run states, acknowledgement records,
and launch recovery. Durable admission, configuration activation, schedule
occurrence tracking, and restore mechanisms still need detailed contracts.

Before the related components are implemented, define environment and secret
references, output capture, metadata and buffer bounds, retention cleanup, and
the private proxy access mechanism. Select the license before publishing a
release. Review a sanitized task inventory before claiming workload compatibility.

This document describes structure and policy. It is not evidence that execution,
recovery, scheduling, or deployment behavior has been tested.
