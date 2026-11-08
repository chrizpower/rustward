# Agent instructions

Before work, read [README.md](README.md), [architecture](docs/ARCHITECTURE.md),
and [Gravity](docs/GRAVITY.md). While present, also read
[requirements](docs/temporary/requirements.md),
[execution contracts](docs/temporary/execution.md), and the
[implementation work list](docs/temporary/implementation.md).
Follow accepted rules and contracts within scope; unaccepted proposals remain
proposals. Read [the glossary](docs/GLOSSARY.md) for domain work and follow its
maintenance instructions in this file. Use ASD-STE100 Simplified Technical English for prose
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

Keep changes within the task. Work directly on main; create a branch only when
explicitly requested. Commit and push only when requested. Stage only task files.
If a branch is requested, use feat/, fix/, docs/, refactor/, test/, or chore/
prefixes.

## Domain language maintenance

### Terminology principles

The user selected these practices. Source attributions are user-supplied;
see [DDD sources](#ddd-sources).

- Foundational DDD (Evans, 2003; Vernon): use precise shared language in code,
  scope it to bounded contexts, and revise it as domain understanding improves.
- Explicit DDD guidance (Evans, Fowler): remove ambiguity and inconsistencies.
- CQRS / EventStorming convention: use imperative command names (`CancelRun`)
  and past-tense event names (`RunCancelled`). These examples do not define
  interfaces or transitions, or require CQRS or event sourcing.
- Rustward-selected optional practice: maintain the formal glossary.
- User recommendation, not foundational DDD: automate terminology checks.
  Checks are not implemented here. Agree terms before selecting check scope;
  review meaning because text matches alone cannot prove consistent usage.

A bounded context defines where a domain model and its language have one
meaning. Rustward has not assigned separate contexts. Record different meanings
with their contexts when needed; resolve inconsistent meanings within a context.
Use agreed terms in code, tests, documentation, and discussions.

### Use and maintain

Use the precise term when a distinction matters, such as task definition versus
input value. Preserve required public interfaces when terminology changes.

After a terminology decision, update its definition and any proposed marker in
the glossary and record its source. Keep implementation contracts and
accepted policy in their source documents. A glossary edit alone must not accept
a proposed behavior. Use examples to resolve ambiguity before introducing new
synonyms. Revise definitions as domain understanding improves. Check affected
code, tests, and documents for inconsistent usage when a definition changes.
Record context boundaries when they become necessary; do not infer them from
module names alone.

During every task that uses domain concepts:

- Read relevant entries, use accepted terms, and avoid listed alternatives within
  their contexts. Preserve
  external interface names and explain their mappings when needed.
- Ask the user about a new concept, ambiguous use, changed meaning, or obsolete
  term. Propose wording and explain its effect with a concrete example. Use a
  structured question tool when available.
- Reuse known answers and authorization. Wait for required answers before
  dependent work. Silence is not acceptance. Continue independent work and
  record deferred meanings and consequences without repeatedly asking.
- Check definitions and relationships with normal and boundary scenarios.
  Compare them with code and report differences. Identify when domain expertise
  is missing; code does not establish intended meaning.
- Record resolved changes during the task, with their source and reason. Keep
  unconfirmed meanings proposed. Check affected references and record remaining
  naming differences after changes or deletions. Do not rename unrelated code or
  change public interfaces without authorization. If edits are outside scope,
  present the proposed glossary change instead.

Determine context boundaries from domain meaning, not repository or deployment
structure. Include technical terms only when they have a project-specific
meaning. No task must complete the whole glossary. These practices do not
require aggregates, repositories, events, or other DDD patterns.

### DDD sources

References for the project terminology principles:

- [Eric Evans, Domain-Driven Design Reference](https://www.domainlanguage.com/ddd/)
- [Martin Fowler, Ubiquitous Language](https://martinfowler.com/bliki/UbiquitousLanguage.html)
- [Vaughn Vernon, Domain-Driven Design Distilled](https://www.informit.com/store/domain-driven-design-distilled-9780134434421)
