# Configuration contracts

Status: Temporary reference for implemented version 2 loading and validation,
and accepted future activation and schedule behavior. Absorb the full
schema, API contract, examples, and compatibility rules into ARCHITECTURE.md,
and relevant domain meanings into GLOSSARY.md before removing this file. Preserve
both contracts and update inbound links. See [folder guidance](README.md).

## Accepted file layout

Source: User decisions in the configuration discussion on 2026-11-08. These
rules replace the single-document layout and explicit task IDs. The user accepted
the exact schema, compatibility, loading, and revision choices below during the
implementation discussion on the same date.

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
The implemented schemas below define the keys, fields, defaults, and version.

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

### Loading and evidence

`config::load(path)` and `rustward validate <rustward.toml>` implement the layout.
The service file must be named `rustward.toml`. Relative definition and storage
paths resolve from its directory, independent of the process working directory.
Absolute paths are allowed. Both definition roots must exist and be directories;
empty roots and empty nested folders are valid. Storage paths need not exist:
loading does not open the database, create log folders, or inspect executables.
Paths must be nonempty and contain no NUL.

Reject all non-TOML entries, invalid folder names, special files, and non-UTF-8
names or file content. This includes hidden files and editor backups. No files
are silently omitted. Use Linux `openat2` with `RESOLVE_NO_SYMLINKS` for every
opened path; reject symlinks in service and definition paths, including roots
and ancestors. Open discovered children relative to held directory descriptors.
Do not use canonicalization to follow a symlink first. This requires Linux with
`openat2` support (available on the selected Debian 13 target); no weaker fallback
is used. Storage paths are configuration values, not loaded files.

Bounds are 1 MiB per file, 16 MiB total file content including the service file,
10,000 discovered entries across both roots, and 32 nested folders below each
root. Each folder and file counts as one entry; root directories do not count.
Reads are bounded before parsing. Limit failures reject the complete set.

The caller must keep files unchanged during loading. The loader reads the
complete service file and both roots twice. It compares file paths, content,
and folder sets and rejects observed changes or any read failure. This is not
an atomic filesystem snapshot and cannot detect every concurrent edit. There
is no claim of reliable live reload. A future service must provide a consistent
candidate set before activation. A failed load returns an error and leaves an
existing caller-owned `Config` unchanged.

Tests exercise nested equal basenames, invalid paths, duplicate identity checks,
symlink files/folders/ancestors, missing and empty roots, loading bounds, invalid
references and fixed values, whole-set failure, observed edits, root relocation,
definition moves, and retained admission identity. Scheduler integration must
still test rename behavior without pause-state transfer or missed-work replay.

## Implemented version 2

Use `cargo run --locked -- validate examples/rustward.toml`. Success returns exit
code 0 and task and schedule counts. Invalid configuration, read errors, and
incorrect CLI arguments return exit code 1. Validation does not run commands,
inspect program availability, contact hosts, or load credentials.

This implements the configuration and input portions of F01, F03, F04, F06,
and F15. Activation, reload, occurrence evaluation, scheduler execution, and
dispatch are not implemented. Service settings are exposed but not connected
to admission, capture, retention, or other runtime components.

## Format change and compatibility

Version 2 deliberately rejects the old single-document version 1 format.
There is no legacy parser or automatic converter. Put `version = 2` only in the
service file. Split `[[tasks]]` entries into individual task files, remove each
task `id`, and select its filename to retain the original ID. Keep input `id`
fields. Move schedules into their own files and use exact task path IDs.
For example, old task ID `country-report` becomes `tasks/country-report.toml`;
`tasks/reports/country-report.toml` would create a different identity.

`config::load(path)` replaces `config::validate(source)`. It returns
`Result<Config, String>`. `Config::arguments` and `Store::admit` still use
`BTreeMap<String, toml::Value>`. The task input types, defaults, choices, and
argument generation rules do not change. YAML remains unsupported.

SQLite schema version 2 and its existing migration are unchanged. No stored
snapshots, arguments, runtime IDs, request keys, fingerprints, or revisions are
rewritten. A retry with an admitted key and original payload returns the original
run even after task removal; a new task path is a changed payload. Existing
version 1 configuration revisions retain their meaning. New configuration
revisions use the separate encoding below. No queued run is recreated.

## Service schema

All object fields are strict: unknown fields and duplicate keys are errors.
The service file has these tables and defaults:

```toml
version = 2

[definitions] # The table and each field can be omitted.
tasks = "tasks"
schedules = "schedules"

[storage] # Both fields are required.
state_database = "/synthetic/state/rustward.db"
log_directory = "/synthetic/logs"

[limits] # The table and each field can be omitted.
active_runs = 1
queued_runs = 100
log_bytes_per_run = 10485760
retained_log_bytes = 1073741824

[retention] # The table and field can be omitted.
days = 30
```

Limits and retention are nonnegative integers. Run counts and days fit `u32`;
byte limits fit `u64` but must also fit TOML's signed 64-bit integer syntax.
Zero is accepted as a configuration value. The loader does not apply limits or
retention, or infer an execution timeout from these settings. Runtime wiring
must preserve the accepted resource and retention contracts. Task files contain
no service settings, environment/secret policy, or prepared runtime selection.

## Task schema

All object fields are listed below. Unknown fields are errors at every level.
Duplicate object keys are errors. Fields are required unless stated otherwise.
Empty task roots are valid. Use `[]` for empty argument and input lists.

| Object | Field | Type and rule |
| --- | --- | --- |
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

Task IDs come only from paths. Input IDs remain single names: start with an
ASCII lowercase letter, followed by lowercase ASCII letters, digits, or hyphens. Each argument object contains exactly one
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

[The synthetic task](../../examples/tasks/synthetic-report.toml) uses paths that
need not exist. The schema has no Ansible-specific behavior.

## Schedule schema

Each schedule file requires `task` (an exact task ID) and `cron` (a string).
`inputs` is an optional table of fixed submitted values; it defaults to an empty
table. No `id`, `version`, pause state, or unknown fields are allowed.

```toml
task = "backup/database"
cron = "0 2 * * *"

[inputs]
name = "Scheduled backup"
```

Validate every fixed input with the task's normal request validation, including
unused inputs. Apply task defaults, reject missing required inputs, and reject
unknown names, incorrect types, NUL, and values outside choices. A missing task
or one invalid schedule rejects the entire configuration.

Cron has five whitespace-separated numeric fields in UTC:

| Field | Range |
| --- | --- |
| Minute | 0–59 |
| Hour | 0–23 |
| Day of month | 1–31 |
| Month | 1–12 |
| Day of week | 0–6; Sunday is 0 |

Each field accepts a standalone `*`, numbers, inclusive ascending ranges, or
comma-separated lists of numbers and ranges. Reject empty list entries, signs,
descending ranges, all `/` steps, names, macros, seconds, years, and special
operators such as `?`, `L`, `W`, and `#`. A wildcard cannot be part of a list.
Syntax validation permits calendar combinations with no occurrence, such as
February 31; it does not search dates. Normalize whitespace between fields to
one ASCII space before revision encoding.

Day of month or day of week must be exactly `*`; both may be `*`. Restricting both
is an error, even if a range covers every value. Thus `0 2 1 * 1` is rejected.
With one unrestricted day field, use the other; with both unrestricted, every
calendar day is eligible. All other fields must match. UTC remains fixed.

| Expression | Meaning |
| --- | --- |
| `0 2 * * *` | Daily at 02:00 |
| `0 2 * * 1-5` | Weekdays at 02:00 |
| `0 2 1 * *` | First day of each month at 02:00 |
| `0,15,30,45 * * * *` | Every quarter hour |

Source: The user accepted this smaller subset in the cron review on 2026-11-08.
It replaces the earlier acceptance of steps and combined restricted day fields.
Use established cron syntax within this subset, with a library parser/evaluator;
do not implement custom occurrence calculations. The
[Debian cron reference](https://manpages.debian.org/trixie/cron/crontab.5.en.html)
documents lists and ranges and the day-field differences that this subset avoids.

The loader enforces the subset before parsing with `croner`. Its tests also check
the library's UTC matching and next occurrences across weekdays, month ends,
year ends, and leap days. These tests do not implement a scheduler or prove
persistent occurrence handling, missed-work policy, or dispatch.

This intentionally narrows version 2 validation: existing files with steps or
both day fields restricted now fail complete-set loading. Replace steps with
explicit values where needed. Unsupported day combinations are not converted
automatically. Existing admitted runs, request fingerprints, and stored revision
bytes remain unchanged. The normalization and encoding of still-valid definitions
do not change; no database migration is required.

## Normalization and revisions

Sort tasks and schedules by their complete derived IDs, independent of discovery
order. Apply schema defaults. JSON object keys are sorted; arrays retain order,
and values retain type. Comments, TOML formatting, and key order have no effect.
An omitted schedule `inputs` table and an explicit empty table normalize equally.
Fixed submitted inputs retain their supplied form; task defaults remain in the
task definition. Input declarations, choices, and arguments retain list order.

Task snapshots retain the version 1 normalized JSON fields, including derived
`id`, and the `rustward-definition-v1` hash prefix. For the same ID and task
content, snapshot and task-revision bytes are unchanged.

New complete-set snapshots are sorted-key JSON objects with `version: 2`,
`tasks`, and `schedules` arrays. Schedule entries contain `id`, `task`, `cron`,
and `inputs`. Hash the exact UTF-8 prefix `rustward-configuration-v2`, followed
by snapshot bytes, with SHA-256. Service settings and root paths are excluded.
Moving a root preserves revisions. Moving a task changes its ID and both its
task revision and the complete-set revision. Moving a schedule changes the
complete-set revision. Stored earlier revisions are never recomputed.


## Request inputs and arguments

Use `rustward::config::load(path)` to obtain a validated `Config`. Call
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

## Dependencies and remaining runtime work

Serde and `toml` provide strict TOML decoding; `rustix` exposes Linux `openat2`
and directory reads. [croner](https://docs.rs/croner/4.0.3/croner/) parses cron
after the subset check. The lockfile
pins version 4.0.3. Its optional time-library support is enabled only for tests,
using `chrono` UTC values without clock or local-time access. The future scheduler
must use this parser/evaluator through the same subset rules. Runtime occurrence
evaluation and scheduling remain unimplemented.

The user accepted schema version 2 and its compatibility decisions before
implementation on 2026-11-08. This does not accept unrelated plan proposals.
[Execution contracts](execution.md) still govern future runtime work. Activation,
reload trigger and consistent-read integration, schedule pause/checkpoint state,
first-installation behavior, missed-occurrence handling, dispatch, and service
resource-limit integration remain unimplemented. No production schedule migration,
live workload, or deployment was performed.
