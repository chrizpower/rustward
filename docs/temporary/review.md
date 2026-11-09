# Administrator review and acknowledgement

Status: Implemented Rust library. On 2026-11-09 the user accepted one note for one
run and local completion before acknowledgement. No CLI, HTTP service, or real
execution is added. Transfer stable interfaces to ARCHITECTURE.md and policy to
GRAVITY.md before removal. Preserve open retention and service integration work.

## Meaning and record

Acknowledgement is the administrator's decision that this run's external
uncertainty need not block new starts. It can accept remaining risk. It does not
prove external completion, change the run result, or retry work.

The administrator supplies one nonempty note. It explains what was checked,
what remains uncertain, and why new starts are permitted. There are no separate
evidence, uncertainty, reason, or Resolved fields. The library cannot validate
the meaning of prose. Notes preserve their exact text, must contain non-whitespace text,
contain no NUL, and fit 4096 UTF-8 bytes.

Rustward automatically retains the run ID, reviewed evidence, and UTC millisecond
acknowledgement time. The evidence includes retained task/runtime IDs and
revisions, phase/result, launch/exit/cleanup evidence, output-failure state, log
accounting, and dispatch/completion times. It copies stored metadata, not task
output, credentials, or whole task definitions. The note can describe external
checks. The record cannot reconstruct transient evidence that was never stored.

## Reading and acknowledging

Methods are on `admission::Store`, so an administrator connection can operate
while the exclusive dispatch session remains open:

- `review(run_id)` returns one consistent `review::Review`, or None when there is
  no pending review for that run.
- `pending_reviews(after, limit)` returns pending reviews in run ID order. The
  page limit is 1–100 entries. Use the last returned ID as `after`; an empty
  page ends the scan. Pages are separate snapshots and do not authorize release.
- `review::new_acknowledgement_id()` supplies a random 64-character lowercase hex
  acknowledgement ID. It is not an admission key or authorization credential.
- `acknowledge(acknowledgement_id, &review, note)` retains the record and clears only that
  run's review block in one immediate SQLite transaction.
- `acknowledgement(acknowledgement_id)` retrieves the retained acknowledgement.

Keep the returned Review unchanged when submitting it. A new acknowledgement must match
both its evidence version and all copied evidence. `ReviewChanged` requires a
fresh read. `NotCompleted` means local cleanup and output handling have not
finished; acknowledgement requires Completed and confirmed local cleanup.
An unknown or no-longer-pending run cannot be acknowledged with a new acknowledgement ID.

There is one run per acknowledgement. Results, first exit evidence, identities,
revision bytes, timestamps, and admitted request fingerprints remain unchanged.
Other runs' review blocks, dispatch pause, execution/log capacity, admission
closure, and the shared storage-failure gate remain in effect. Acknowledgement
neither starts work nor frees a slot. Local completion already released its slot.

## Evidence changes and retries

Schema version 5 introduced a nonnegative integer evidence version. Updates to launch,
cleanup, review, output, accounting, or lifecycle evidence while review is pending
advance it. Repeated writes can advance the version even if displayed values
are equal. Recovery advances pending unfinished evidence. Overflow fails instead
of wrapping. Clearing review through acknowledgement does not advance it.
New uncertainty raises a later version; an old acknowledgement cannot clear it.

Exact retries with the same acknowledgement ID, Review, and note return the original
acknowledgement before current-state validation, including after local history
removal. A changed payload with the same acknowledgement ID returns PayloadConflict.
A competing new acknowledgement ID for an acknowledged review returns ReviewChanged.
Keep the acknowledgement ID and submitted Review until the response is known.

The stored record uses encoding 1. Acknowledgements have no deletion or editing
API in this slice. Retention reclamation and its retry guarantees remain future
work. The copy preserves reviewed metadata if run history is later removed.

## Failure, compatibility, and verification

Version 1–5 migrations preserve runs, permanent admissions, fingerprints,
identities, snapshots, revisions, sequence high-water marks, and unknown historical
times. Existing review flags remain set; pending legacy rows receive evidence
version 1. Migration does not acknowledge work. Older binaries reject version 6. Schema version 6 adds optional `launch_failure`
evidence. Encoding 1 records without this field still read as None; empty values
are omitted when encoding new records. Stored acknowledgements are not rewritten.

Insert and release must both succeed and commit before acknowledgement succeeds.
A failed or ignored write rolls back both changes. Storage errors close the
handle and set its shared Safety gate. Busy can be retried: this transaction has
no external side effect. A lost reply after commit is resolved with the same
acknowledgement ID and payload. Do not infer commit status from a lost reply.

Synthetic tests cover local completion gates, stale and modified evidence,
competing requests and matching retries, another run's review, pause/capacity and
storage gates, write rollback, lost replies, restart, retained history, bounded
notes/pages, and version 4 migration rollback. These tests do not prove storage
durability after host power loss, external outcomes, or transport authentication.

The future service must authenticate the administrator, present the current
review and output evidence, connect shared Safety handles, and preserve acknowledgement
IDs across retries. Real execution, runtime/host binding, automatic
capture orchestration, systemd host checks, retention cleanup, scheduler execution,
live reload, production migration, and deployment remain outside this slice.
