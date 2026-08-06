# ADR 0005: Per-user Runtime and Gateway

- Status: Accepted
- Date: 2026-07-28
- Amended: 2026-07-30

## Context

The supported Windows release must work for a standard user without UAC. Sandbox repair and local
Channel ingress must survive app or login restarts without installing machine-wide services.

## Decision

1. NSIS installs for the current user. Sandbox profiles, markers, logs, Chromium, portable Git and
   managed sidecars live under versioned per-user data/runtime roots.
2. Startup and repair atomically restage fixed-hash files, create or resolve the current user's
   AppContainer Profile, apply only the required ACLs and rerun four-axis attestation. No normal path
   uses `runas`, a Windows Service, system Git discovery or system Git ACL changes.
3. Runtime/marker/Profile SID drift narrows readiness immediately. Repair is idempotent; it does not
   run concurrently with Agent, Process, Workspace or stdio MCP activity.
4. Gateway is the same signed desktop executable in `--gateway` mode. HKCU Run starts it at login;
   enabling startup also starts the current session. Loopback binding provides a single persistent
   instance, heartbeat reports liveness, and restart reconciliation returns claimed ledger rows to
   safe retry states.
5. A per-user bearer token protects loopback webhook and desktop wake IPC. The token, Connector
   secrets and page/session secrets are excluded from ordinary SQLite and Audit.

## Consequences

- Administrator accounts, elevated installers, administrator runners and elevated-window control
  are outside the current release acceptance boundary.
- Failure to prove Runtime integrity, Workspace ownership or any enforcement axis keeps all
  side-effecting Hosts fail closed and returns a stable actionable error.
- Workspace, AppServer and Gateway remain local per-user processes without a Windows Service.
  Production Channel adapters are limited to WeCom, DingTalk and Feishu and must preserve this
  local Gateway boundary.
