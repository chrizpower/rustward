# Task configuration version 1

Use `cargo run --locked -- validate examples/tasks.yaml` to validate a file.
The installed binary accepts `rustward validate <config.yaml>`. Success returns
exit code 0. Invalid configuration, read errors, and incorrect CLI arguments
return exit code 1. Validation reads one UTF-8 YAML document. It does not run
commands, inspect program availability, contact hosts, or load credentials.

This slice defines the task contract for F01, F03, F04, and F15. It implements
configuration validation only. It does not implement request input validation,
argument generation, activation, reload, schedules, or execution.

## Schema

All object fields are listed below. Unknown fields are errors at every level.
Duplicate object keys are errors. Fields are required unless stated otherwise.
An empty task list is valid. Use `[]` for empty argument and input lists.

| Object | Field | Type and rule |
| --- | --- | --- |
| Configuration | `version` | Integer; must be `1` |
| Configuration | `tasks` | List of task objects; task IDs must be unique |
| Task | `id` | Stable ID |
| Task | `label` | String with non-whitespace text; no NUL |
| Task | `executable` | Absolute program path; no NUL |
| Task | `working_directory` | Absolute directory path; no NUL |
| Task | `arguments` | Ordered list of argument objects |
| Task | `inputs` | List of input objects; IDs must be unique within the task |
| Argument | `literal` | String, including an empty string; no NUL |
| Argument | `input` | ID of an input in this task |
| Input | `id` | Stable ID |
| Input | `type` | `string`, `integer`, or `boolean` |
| Input | `required` | Optional boolean; defaults to `false` |
| Input | `default` | Optional scalar of the declared type; explicit null is invalid |
| Input | `choices` | Optional nonempty list of distinct values of the declared type |

IDs start with an ASCII lowercase letter. Remaining characters are ASCII
lowercase letters, digits, or hyphens. Each argument object contains exactly
one field: `literal` or `input`. Strings must be YAML strings. Quote strings
that could be read as another YAML scalar type. Integers must fit a signed
64-bit value. Booleans are `true` or `false`. String inputs cannot contain NUL.
Defaults must match the declared type and belong to `choices` when it is set.

A required input can have a default. The default supplies its value when no
value is submitted. A referenced input must be required or have a valid default.
This rule prevents an absent optional input from changing argument positions.
An unused input is permitted. Choices can constrain any supported input type.

The argument contract reserves one argument for each list entry. A literal is
unchanged. An input supplies its complete value as one argument; strings remain
unchanged, integers use decimal form, and booleans use `true` or `false`.
There is no interpolation, splitting, implicit flag, conditional omission,
shell expansion, or environment substitution. The validator checks references;
argument generation and submitted value validation are future work. A program
can still interpret an argument as an option. Task owners must use program
options such as `--` where needed. Validation does not inspect program behavior.

[The synthetic example](../examples/tasks.yaml) uses paths that need not exist.
The generic schema contains no Ansible-specific fields, secrets, schedules,
runtime selection, environment configuration, or execution limits.

## Dependencies and open decisions

Use Serde derive for typed decoding and `serde_yaml_ng` 0.10 for YAML parsing.
The lockfile records exact dependency versions. Serde supports rejection of
[unknown fields](https://serde.rs/container-attrs.html). The parser documents
[YAML 1.1 support](https://docs.rs/serde_yaml_ng/0.10.0/serde_yaml_ng/).
Tests check the schema rules instead of assuming scalar conversion is safe.

This version is the first implementation contract, authorized by the task
configuration slice. It does not accept unrelated plan proposals. Review schema
compatibility and migration before changing version 1 or publishing a release.
Before execution, define prepared runtime identity and path ownership (P03),
submitted input validation, environment and secret references, task resource
controls, and metadata and configuration size bounds (P07). Systemd manager,
permissions, state transitions, and recovery remain separate implementation
gates. Workload compatibility still needs a sanitized task inventory. License
selection remains a release gate. No live workload has been validated.
