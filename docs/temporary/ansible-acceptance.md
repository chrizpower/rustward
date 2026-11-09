# Ansible acceptance test

Status: Planned acceptance test; not implemented or run. The user requested this
planning artifact on 2026-11-09. It tests the accepted generic execution rules
with the main workload. It does not accept new execution policy or authorize
production access, schedule migration, or deployment.

## Scope and authority

Prove that a synthetic Ansible playbook can complete through manual admission,
durable dispatch, real systemd supervision, output capture, and durable
completion. Run an ordinary command through the same path to check C01.
Use [Gravity](../GRAVITY.md) for policy and the
[execution contracts](execution.md) for required behavior. The
[implementation work list](implementation.md) owns integration work.

This is an early execution integration milestone, before UI and scheduler
integration. A successful direct invocation of ansible-playbook, a fake
supervisor test, or configuration validation alone does not pass it. Passing
does not establish compatibility with all existing playbooks; the sanitized
Semaphore workload review remains a separate gate.

## Prerequisites and fixture

- Use an isolated Debian 13 host with systemd 257, a dedicated unprivileged
  user manager, and verified unit permissions. Follow the dispatch integration
  gates; do not connect the devbox to a production bus.
- Prepare fixed Ansible, Python, collection, and playbook revisions outside
  execution. Record their versions and runtime identity. Do not install or fetch
  dependencies during the run.
- Keep fixture source and reusable test inputs in the persistent checkout.
  Use only synthetic inventory, inputs, and credentials. Keep evidence under
  `$HOME/artifacts/rustward/ansible-acceptance/<test-id>/`.
- Start with a local playbook that writes a known value to a dedicated test
  directory and checks its content. Add an isolated SSH target for remote
  effects, connection failures, and interruption tests. Give each test its own
  target data and restore that data between cases.
- Include a string with spaces and quotes, an integer, and a boolean. Check the
  values and types inside the playbook. Use a synthetic secret marker to check
  that credential handling does not expose it in retained inputs, arguments,
  diagnostics, or output.
- Use explicit synchronization points for failure injection. Wait for recorded
  states or target markers rather than relying only on elapsed sleep time.

Before implementing the fixture, specify runtime path selection, separate
writable run directories, environment and secret references, and unattended
credential access. Specify how typed inputs reach Ansible and how optional
flags are selected. The current argument contract does not provide conditional
flags or structured variable generation. A workload wrapper can supply these
features outside the core; any schema or policy change needs separate review.
Record unresolved choices here without treating them as accepted behavior.

## Test procedure and pass criteria

For each case, record the fixture revision, runtime identity, submitted
non-secret inputs, run IDs, trigger, expected result, observed result, and
evidence paths. Inspect both Rustward state and the test target where relevant.

| ID | Action and conditions | Pass criteria |
| --- | --- | --- |
| A01 | Submit the local playbook through manual admission; repeat the same submission key and payload | Both submissions identify one run. The playbook starts once, receives the expected typed values, writes the expected content, and completes with Success after local cleanup and output capture |
| A02 | Run an ordinary command through the same admission and execution path | It completes with the expected result and output. No Ansible-specific branch or dependency is added to the execution core |
| A03 | Run the playbook against the isolated SSH target using synthetic credentials | The expected target change is verified. Host-key verification remains enabled, no interactive input is needed, and the synthetic secret marker is absent from retained service data and output |
| A04 | Cause a playbook task to fail; separately make the target unreachable | Each run records Failure with exit evidence and useful output. Success is not reported. Local cleanup is confirmed before its execution slot is released |
| A05 | Cancel a queued run; separately cancel and time out active remote work at a known synchronization point | The queued run never starts. Active runs retain Cancellation or Timeout as applicable. All local children end before slot release. Uncertain external work blocks further dispatch pending review |
| A06 | Kill the Rustward service after remote work starts, with another run queued; restart it | Recovery preserves queued work, does not automatically retry interrupted work, and records Interruption where the normal result cannot be established. Uncertain external work blocks new starts; acknowledgement requires local completion and review of current evidence |
| A07 | Start remote asynchronous work and interrupt its local controller while the remote action remains observable | Evidence distinguishes local cleanup from remote completion. Dispatch remains blocked while external work is uncertain. Review records what happened remotely or explicitly accepts remaining risk; it does not turn the interrupted result into Success |
| A08 | Admit work with runtime A, then select runtime B before dispatch; repeat while A is active | Queued and active runs retain A and its fixed code and dependencies. New admissions can select B. Removing a selected runtime causes explicit failure, with no substitution of B |
| A09 | Generate output beyond the configured per-run limit | Retained output stays within the limit, truncation is recorded, output continues to drain, and the playbook does not block on full output pipes |
| A10 | Inject a log-storage failure during active remote work, with another run queued | Admission and dispatch close, local stopping starts without requiring another successful database write, and no unsupported durable success is reported. Uncertain external work requires review under P06 |

A01 and A02 establish the first local integration result. A03 through A10 are
required before this acceptance test is complete. Existing generic tests still
own broader crash-window, retention, concurrency, and storage coverage; these
workload cases do not replace them. UI and scheduled requests need later
integration tests against the same execution path.

## Evidence and remaining work

All cases are currently **not run**. No executable acceptance harness or fixture
exists yet. Implement them with the execution integration work; do not add a
placeholder test that reports success without starting Ansible through Rustward.

For each execution, record the exact command, host baseline, dependency versions,
case results, and relevant state, output, and target observations. Preserve
failure evidence before cleanup. Record blocked and skipped cases with reasons;
they do not count as passes. Keep credentials out of the evidence.

Remaining work is to resolve the fixture prerequisites, implement the harness,
run each case, and review the results. Confirm how representative the synthetic
fixture is during the separate workload compatibility review.

## Completion and removal

Keep this document until the cases have executable coverage and retained
acceptance evidence, or unresolved work has an explicit maintained destination.
Before removal, transfer lasting integration behavior and evidence references
to ARCHITECTURE.md, policy decisions to GRAVITY.md, and agreed domain meanings
to GLOSSARY.md. Preserve useful evidence and update inbound links.
