# Temporary documentation

All supporting documents in this folder are temporary, including references
that currently specify accepted or implemented behavior. Temporary location does
not weaken their authority or permit loss of decisions, contracts, or evidence.

The long-lived documents are README.md and AGENTS.md at the root, and
[ARCHITECTURE.md](../ARCHITECTURE.md), [GRAVITY.md](../GRAVITY.md), and
[GLOSSARY.md](../GLOSSARY.md) in docs/. Absorb useful content into those documents
before removing the temporary source. Avoid copying whole documents when a
concise section can preserve the meaning and required detail.

| Temporary file | Content destination and removal condition |
| --- | --- |
| [Requirements](requirements.md) | Preserve requirement IDs, accepted scope, acceptance criteria, and unresolved proposals in ARCHITECTURE.md or GRAVITY.md; delete after transfer and link updates |
| [Execution contracts](execution.md) | Put lasting behavior and lifecycle details in ARCHITECTURE.md, policies and exceptions in GRAVITY.md, and terms in GLOSSARY.md; delete after all contracts and open decisions are transferred |
| [Configuration](configuration.md) | Absorb the implemented schema, API contract, examples, and compatibility rules into ARCHITECTURE.md; put domain meanings in GLOSSARY.md; delete after the reference is complete at its destination |
| [Implementation work](implementation.md) | Complete or explicitly defer work; preserve lasting detail in the uppercase documents and verification in tests or acceptance records; delete when no unique work remains |
| [Migration preparation](migration.md) | Accept or cancel migration; preserve lasting application requirements in ARCHITECTURE.md or GRAVITY.md and operational procedures/evidence in the deployment repository through authorized work; then delete |

Each file must state its scope, status, removal condition, and destination.
Before removal, preserve useful evidence, resolve or transfer open items, and
update inbound links. Preserve accepted/proposed and implemented/planned status
when moving content. Do not copy secrets into documents.

Remove this index when the folder has no remaining purpose. Update README.md and
AGENTS.md as temporary files are absorbed; do not leave required reading links
to deleted files.
