# ADR 0003: Shared Agent Scheduler and persistent schedule authorization

- Status: Accepted
- Date: 2026-07-26

## Context

Hachimi needs prompt-based recurring tasks without introducing a second Agent runtime or silently
granting a background model the privileges of an earlier interactive Run. The scheduler must keep
working while the tray process is alive, survive application restarts, and avoid duplicate
invocations.

The product-behavior reference is Codex
[Scheduled tasks](https://developers.openai.com/codex/app/automations), especially standalone and
chat-bound runs, Skills/Plugins, Worktree isolation, unattended permissions, and reviewable results.
OpenClaw commit `f6d456235cf011004f7cffc71a95acf6fbf1fa0a` is only the implementation reference for persistent
schedule definitions, the single-nearest-timer, isolated invocation, task ledger, reconciliation,
and delivery boundaries. No OpenClaw Agent Core, standing-order authorization, or side-effecting Turn
replay is adopted. Any later source adaptation must first use the fixed commit and be registered in
`HARNESS_AGENT_SOURCE_PROVENANCE.md`.

## Decision

1. `SchedulerService` calculates occurrences with an injected clock and launches them through the
   same `AgentRunFactory`, `AgentRunExecutor`, and `AgentExecutorRegistry` used by interactive Runs.
2. A `Standalone` trigger creates a fresh `TaskRun`, `Session`, `Run`, generation and transient Run
   grant. A `SessionContinuation` trigger binds a fresh `TaskRun`/`Run` to an existing Session lane.
   Scheduler state never contains an Agent Tool Loop.
3. The database is authoritative. A unique invocation key identifies a schedule occurrence. The
   nearest enabled occurrence owns the single wake-up timer; startup reconciliation recomputes it.
4. At most one invocation per Schedule may be active. A later overlapping occurrence is recorded as
   `Skipped`. Background concurrency is separately limited and interactive work has priority.
5. A user-signed `ScheduleGrant` is versioned independently from general configuration. Name,
   prompt, timing, and delivery edits do not revoke it. Profile, context, execution target, tools,
   Skills, MCP tools, file/process permissions, or external targets increment `permission_revision`
   and require reauthorization.
6. Every invocation intersects the stored authorization scope with current Policy, Sandbox and Host
   readiness. Schema, Skill, MCP or Host identity changes can reduce access or produce
   `NeedsAttention`; they can never expand authorization.
7. Background Approval, Elicitation, or UserInput outside the exact grant does not wait indefinitely.
   The TaskRun becomes `NeedsAttention`, and the user may create a new interactive continuation Run.
8. Secrets and OAuth tokens remain in the OS Keyring. Schedule and Task tables store opaque
   references or hashes only.
9. System notifications contain only the task name and terminal category. Delivery failure is
   tracked independently from execution success.
10. Completely exiting the process stops scheduling. Tray/background process lifetime is the
    supported execution lifetime; system wake and OS service installation are deferred.
11. A continuation may read the Session's persisted compacted context, but it recaptures
    StepContext, ToolPlan, ScheduleGrant, contribution revisions and Host readiness. It never
    restores Approval, UserInput secret, temporary Grant, Browser observation, Computer frame,
    process lease or MCP session. Maximum occurrences, end time, stop-after-success and user disable
    are deterministic stop conditions; every trigger appends a thread heartbeat Item.

## Consequences

- General and Project scheduled work reuse all Agent lifecycle, Compaction, Approval, Sandbox,
  Diff, Evidence, and recovery semantics.
- `ScheduleGrant` is neither an Approval nor an authority source available to model output, prompt,
  Skill, MCP, Hook, or Elicitation.
- Removing a Schedule retains TaskRun history by default. Dirty Worktree cleanup is a separate,
  reviewable operation.
- Clock, notification, model factory, and Run launcher boundaries must be injectable for deterministic
  DST, misfire, restart, and duplicate-trigger tests.
