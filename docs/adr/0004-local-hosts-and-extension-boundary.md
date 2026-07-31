# ADR 0004: Local Hosts and extension boundary

- Status: Accepted
- Date: 2026-07-28
- Amended: 2026-07-30

## Context

Browser, desktop control, Plugins/Connectors and external Channels must extend the product without
creating a second Agent loop or allowing untrusted content to manufacture authority.

## Decision

1. `browser.*`, `computer.*`, `plugin.*`, `connector.*`, `channel.*` and `gateway.*` are typed
   AppServer domains. Model-visible tools still execute through the single `AgentRunExecutor`,
   StepContext, Policy, Approval, Capability Grant and Sandbox chain.
2. Browser supports a managed Chromium isolated Profile and an explicitly paired Chrome extension.
   Origin/capability grants, task-owned tabs and observation IDs fence every action. Upload/download
   use short-lived Session-bound tokens and isolated storage.
3. Computer uses Windows Graphics Capture for Observe and allowlisted `SendInput` for Act. Every
   action binds a Frame, App and Window fingerprint. Elevated/protected desktops, Hachimi windows,
   background windows and stale Frames fail closed.
4. Plugin bundles are local, content-addressed and manifest-bounded. Lifecycle changes and upgrades
   never silently expand permissions. Connector credentials live in Windows Credential Manager;
   SQLite stores only `secret_ref`, revision and metadata.
5. `sample-crm` is the deterministic Connector acceptance fixture. Plugin distribution remains
   local and content-addressed; the complete contribution update/rollback/reconciliation lifecycle
   is prepared for implementation.
6. Channel/Gateway only authenticates, routes and persists ingress/outbox. It cannot approve a
   high-risk action and owns no model loop. `loopback-webhook` and `mock-poll` are deterministic
   acceptance fixtures. Production Connector/Channel work is limited to WeCom, DingTalk and Feishu.
7. Page text, DOM, downloaded content, Connector data and Channel messages are untrusted external
   content. They cannot alter system instructions, grants, approvals or pinned revisions.

## Consequences

- Browser screenshots and Computer frames may enter only the current in-memory model request and are
  excluded from Transcript, SQLite, Audit and durable side-effect results.
- Plugin/Connector and Channel/Gateway status is accurately described as “framework and samples
  complete; full contribution lifecycle and WeCom/DingTalk/Feishu integrations prepared.”
- Local Host kill switches can reduce availability but cannot create authority.

## Source boundary

Codex public product behavior is the primary permission/interaction reference. OpenClaw fixed commit
`f6d456235cf011004f7cffc71a95acf6fbf1fa0a` is a behavior reference for Channel routing and durable
delivery. Current Local Host implementations are original Hachimi code; exact derivations, if any,
must be registered before adaptation.
