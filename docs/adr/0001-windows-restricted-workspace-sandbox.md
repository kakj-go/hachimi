# ADR-0001: Windows restricted Workspace Sandbox

- Status: Accepted
- Date: 2026-07-25
- Owners: Harness Agent / Desktop

## Context

The Workspace Worker is already a separate process, but it currently inherits the desktop user's
Windows access token. Process separation alone does not enforce the persisted filesystem, process,
or network grants. Hachimi must not report an enforced sandbox until a runtime probe proves the OS
boundary.

## Decision

1. Windows is the first enforced backend. `ReadOnly` may run with an explicit degraded report;
   workspace writes and process execution fail closed unless the backend is attested.
2. MSI and NSIS installers provision the sandbox identity and policy marker through a dedicated,
   elevated setup helper. Debug and portable builds expose the same helper as an explicit
   administrator action.
3. The marker is not an attestation. Startup resolves the installed AppContainer identity and SID,
   verifies the policy version, creates an AppContainer process security context, and runs
   filesystem/process/network canaries before setting `SandboxStatus::Enforced`.
4. Every side-effecting Workspace Worker is launched through `SandboxBackend::spawn_restricted`.
   The child and all descendants are assigned to a kill-on-close Job Object, inherit only explicit
   stdio handles, receive an allowlisted environment, and use a Run-scoped temporary directory.
   Windows process creation registers both `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES` and
   `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`; the latter contains only child-side stdin/stdout/stderr
   handles created for that launch. With no stdio, handle inheritance is disabled. An inheritable
   sentinel handle that is deliberately omitted from the list must remain unusable in the child.
5. C2.1 enforces deny-all network access. Host/protocol allow rules remain a separate C2.2 change.
6. Path authorization uses Windows-native normalization plus final-handle verification. UNC,
   device namespaces, alternate data streams, escaping reparse points, and unsupported volumes are
   rejected for the first release.
7. `SandboxRuntimeManager` is the live readiness authority. Workbench can refresh status or launch
   the fixed setup helper through Windows `runas`; a repair immediately narrows all enforcement
   flags, then reruns full attestation. Workspace mutation, Process spawn, and stdio MCP startup are
   mutually exclusive with repair, so a check-then-start race cannot retain the old authority.
8. A Run keeps its creation-time Sandbox snapshot, while every side effect intersects it with the
   current runtime report. Repair can narrow an active Run but cannot upgrade it.

## Consequences

- The Agent kernel still receives no filesystem or process handle.
- `SandboxCapabilityReport` is the only UI source of readiness truth.
- A setup or canary failure disables write/Exec instead of silently falling back to the desktop
  user token.
- Real Windows smoke tests require an administrator-capable NTFS runner and remain distinct from
  deterministic desktop UI tests.
- The protected administrator gate runs setup/attestation, handle sentinel, Workspace Worker,
  Agent Exec, MCP stdio, Terminal/ConPTY, Desktop/Toast, SystemClock soak, and portable restart
  through one `pnpm test:windows:release` entry point; external pull requests cannot run it.

## Source boundary

The filesystem, process-tree, setup, attestation, and fail-closed boundaries were reviewed against
OpenAI Codex at fixed commit `4c43465133428898aa84f0bfc02c306ed65fb66a` using fixed-object
inspection. Hachimi's AppContainer identity, launch plumbing, installer integration, and runtime
canaries are original implementations. The refreshable setup orchestration in
`crates/hachimi-sandbox/src/runtime_manager.rs` selectively adapts Codex setup-orchestrator control
flow and is registered per file in the provenance ledger; it does not embed Codex Core or reuse
Codex product prompts.
