# Configuration contracts

Status: Temporary reference for implemented version 1 behavior and the accepted
future file layout. The file layout is not implemented. Absorb the full
schema, API contract, examples, and compatibility rules into ARCHITECTURE.md,
and relevant domain meanings into GLOSSARY.md before removing this file. Preserve
both contracts and update inbound links. See [folder guidance](README.md).

## Accepted file layout

Source: User decisions in the configuration discussion on 2026-11-08. These
rules replace the single-document layout and explicit task IDs for the future
loader. They do not change the current CLI or version 1 parser by documentation
alone. Exact schemas and compatibility remain open below.

### Files and ownership

Use one `rustward.toml` for service configuration, one TOML file per task,
and one TOML file per schedule. Allow nested folders under both definition roots:

```text
rustward.toml
tasks/
  backup/
    database.toml
    files.toml
schedules/
  backup/
    database-nightly.toml
```

Service configuration owns definition locations, state and log paths, global
execution and queue limits, and retention settings. Task files own executable,
working directory, arguments, label, and input schema. Schedule files select a
task, fixed inputs, and UTC times. Several schedules can select the same task.
Exact TOML keys, tables, required fields, and version placement are not yet set.

Git-managed TOML is the sole source of task and schedule definitions. The web UI
does not create or edit these definitions. SQLite retains runtime state,
including occurrence records, evaluation state, and schedule pause state; it is
not a second source of schedule definitions. Operational controls remain
separate from definition editing.

### Path identity

Derive each task or schedule ID from its path relative to its definition root.
Remove the final `.toml` suffix and use `/` between path components. Do not put a
second ID field in the definition file. Task and schedule IDs have separate
scopes; IDs must be unique within each complete set.

Every folder name and filename stem must match `[a-z][a-z0-9-]*`: start with a
lowercase ASCII letter, then use only lowercase ASCII letters, digits, or
hyphens. Use the lowercase `.toml` suffix. Empty components, `.` and `..`,
uppercase letters, underscores, and backslashes are not valid ID components.
Symlinks are not allowed for definition files or folders; discovery must not
follow them. Input IDs remain single names under the current input rules.

| Definition file | ID |
| --- | --- |
| `tasks/backup/database.toml` | Task `backup/database` |
| `tasks/maintenance/database.toml` | Task `maintenance/database` |
| `schedules/backup/database-nightly.toml` | Schedule `backup/database-nightly` |

A schedule selects the exact task ID, for example `backup/database`. Moving a
whole definition root preserves IDs. Renaming or moving a file within its root
changes its ID and means removal plus addition. Moving a folder changes every
ID below it. Update affected references in the same configuration update.
Folders group definitions; they do not introduce inherited task settings.

### Validation and retained work

Validate the complete candidate set before activation, including unique IDs,
path rules, task definitions, schedule task references, and fixed input values.
Reject an invalid update as a whole and retain the previous active set. Do not
silently omit an invalid definition or activate only the valid files. Activation
and reload are not implemented; the trigger and consistent-read mechanism remain
open. This contract accepts whole-set validation and activation, not a file
watcher or a specific reload command.

Queued and active runs retain their accepted definitions and original identities
after edits, moves, or removal. Keep existing request-key comparison rules: a
retry with the original task ID can return its original run after removal;
changing the task ID changes the request payload.

A renamed schedule is a new schedule. Do not automatically transfer pause state
or occurrence history to its new ID. Preserve the old records under their
original identity. Apply first-installation behavior to the new schedule and
never replay missed occurrences. Exact initial pause and checkpoint mechanisms
remain to be specified. Editing a schedule at the same path preserves its ID
and must not make a handled occurrence eligible again.

### Implementation decisions and evidence

Before changing the parser, specify the service, task, and schedule schemas,
versioning, CLI and Rust API changes, and whether the old single-document format
is rejected or supported during migration. Specify definition-root resolution,
missing and empty roots, non-TOML entries, discovery bounds, and consistent
reads during concurrent file edits. Enforce the no-symlink rule during loading;
resolve root and ancestor checks without following definition symlinks.
Do not infer these details from the directory example.

Define deterministic normalization for the complete set, including derived IDs,
and its revision encoding. Directory traversal order must not define identity.
Preserve stored snapshots, revision meanings, request fingerprints, and admitted
runs; do not rewrite history to use new paths. Service-setting participation in
configuration revisions remains open. Review schema and API compatibility before
implementation; the earlier YAML-to-TOML conversion below does not settle it.

Required checks include nested paths with equal basenames, invalid components,
file and folder symlinks, duplicate IDs, unresolved schedule references, invalid
fixed inputs, complete rejection of invalid updates, root relocation, file and
folder moves, and retained admission identity after task removal. Schedule
integration must also check rename behavior without pause-state transfer or
missed-occurrence replay. These are required future checks, not test results.

## Implemented version 1

Use `cargo run --locked -- validate examples/tasks.toml` to validate a file. The
installed binary accepts `rustward validate <config.toml>`. Success returns exit
code 0. Invalid configuration, read errors, and incorrect CLI arguments return
exit code 1. Validation reads one UTF-8 TOML document. It does not run commands,
inspect program availability, contact hosts, or load credentials.

This implemented contract covers the configuration and input portions of [plan
requirements F01, F03, F04, and F15](requirements.md#functional-requirements).
It implements configuration validation, request input validation, and argument
generation. It does not implement activation, reload, schedules, or execution.

## Format change and compatibility

The user requested replacement of YAML with TOML. This deliberately replaces
the earlier YAML version 1 contract. Rustward accepts TOML only; renaming an old
file does not convert its content. The schema keeps `version = 1` and the same
field names, supported input types, defaults, choices, and argument rules.
The file format and Rust value type change: callers of `Config::arguments` and
`Store::admit` must use `BTreeMap<String, toml::Value>`. TOML has no null or
unsigned integer value. Out-of-range integers fail during parsing. Floating
point and datetime values exist in TOML but are not accepted input values.

Convert task lists to `[[tasks]]` tables. Set `arguments` and `inputs` to arrays
of inline tables, as in the example, or use arrays of tables. Quote every string;
write key/value pairs with `=`. Use `tasks = []` for an empty configuration.
For example:

```toml
version = 1

[[tasks]]
id = "country-report"
label = "Country report"
executable = "/synthetic/bin/report"
working_directory = "/synthetic"
arguments = [{ input = "country" }]
inputs = [{ id = "country", type = "string", default = "NO" }]
```

SQLite schema version 2 and its migration remain unchanged. Existing stored JSON
snapshots, submitted values, arguments, keys, and request fingerprints remain
valid. Equivalent validated definitions produce the same normalized JSON and
revisions. New TOML values use the same scalar fingerprint encoding. No queued
run is rewritten or recreated during this format change.

## Version 1 schema

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
| Input | `default` | Optional scalar of the declared type; omit it when no default is needed |
| Input | `choices` | Optional nonempty list of distinct values of the declared type |

In the implemented version 1 schema, task and input IDs start with an ASCII
lowercase letter. Remaining characters are ASCII lowercase letters, digits, or
hyphens. Each argument object contains exactly one
field: `literal` or `input`. Strings must be quoted TOML strings. For example,
`"NO"`, `"yes"`, and `"on"` are strings; bare `NO`, `yes`, and `on` are syntax
errors. Booleans are only `true` and `false`. Integers must fit a signed 64-bit
value. String inputs cannot contain NUL.
Defaults must match the declared type and belong to `choices` when it is set.

A required input can have a default. The default supplies its value when no
value is submitted. A referenced input must be required or have a valid default.
This rule prevents an absent optional input from changing argument positions. An
unused input is permitted. Choices can constrain any supported input type.

The argument contract reserves one argument for each list entry. A literal is
unchanged. An input supplies its complete value as one argument; strings remain
unchanged, integers use decimal form, and booleans use `true` or `false`. There
is no interpolation, splitting, implicit flag, conditional omission, shell
expansion, or environment substitution. Configuration validation checks
references. Request validation checks all submitted values before argument
generation. A program can still interpret an argument as an option. Task owners
must use program options such as `--` where needed. Validation does not inspect
program behavior.

[The synthetic example](../../examples/tasks.toml) uses paths that need not exist.
The generic schema contains no Ansible-specific fields, secrets, schedules,
runtime selection, environment configuration, or execution limits.

## Request inputs and arguments

Use `rustward::config::validate(source)` to obtain a validated `Config`. Call
`Config::arguments(task_id, &values)` with a `BTreeMap<String,
toml::Value>`. It returns `Result<Vec<String>, String>`. The
configuration fields are private, and only configuration validation can
construct a `Config`. The method does not change the configuration or submitted values. Internal
request input validation retains typed input values, including defaults, until
argument generation converts referenced values to strings. Unused input values
are validated but produce no arguments. See the [implementation overview](../ARCHITECTURE.md#current-implementation)
for the private types; their names do not change TOML field names.

Task selection uses an exact ID match. Unknown task IDs and undeclared input
names are errors. Each declared input uses its submitted value, or its default
when absent. TOML has no null value; omit an input to select its default. A
required input without either value is an error. An absent optional input
without a default is permitted only when no argument references it, as required
by the configuration schema. Unused inputs still receive type and choice checks.

Values must have the declared type, without conversion. Integers must fit `i64`;
floats, datetimes, arrays, tables, and numeric strings are invalid. Choice values must match exactly.
Strings preserve empty values, spaces, Unicode, and shell syntax. NUL is
invalid. Each configured literal or input produces exactly one argument, in list
order. Repeated input references produce repeated arguments. Errors identify the
task or input and do not include submitted values. The map interface does not
define a request wire format or duplicate-key handling for a future transport.

## Dependencies and open decisions

Use Serde derive for typed decoding and the `toml` 1.2 crate with Serde support
for TOML 1.1 parsing. The lockfile records exact dependency versions. The
[parser documentation](https://docs.rs/toml/1.2.1/toml/) and
[TOML specification](https://toml.io/en/v1.1.0) define syntax. Tests check strict
types, duplicate fields, unsupported values, and quoted ambiguous-looking text.

The implemented version 1 schema is the first implementation contract,
authorized by the task configuration slice. It does not accept unrelated plan proposals. Review schema
compatibility and migration before changing version 1 or publishing a release.
[Execution contracts](execution.md) define accepted future behavior. The
[temporary implementation work list](implementation.md#remaining-specifications-and-evidence)
tracks outstanding integration gates. This schema has no runtime or schedule
fields. The separate [admission library](../ARCHITECTURE.md#manual-admission-library)
retains a caller-selected runtime identity without changing this schema.
Configuration validation alone implies no admission or execution behavior. No live workload
has been validated.
