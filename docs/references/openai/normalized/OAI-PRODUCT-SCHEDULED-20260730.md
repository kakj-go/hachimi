# Scheduled Tasks behavior snapshot

- Reference ID: `OAI-PRODUCT-SCHEDULED-20260730`
- Product behavior: scheduled tasks run in the background, may be standalone or continue an existing chat, can use skills/plugins, and can run in a local project or isolated worktree.
- Capability boundary: unattended runs use explicit sandbox/approval policy; narrow access is preferred, and organization requirements can further restrict the allowed mode.
- Hachimi mapping: ScheduleDefinition/Grant/TaskRun ledger, standalone and Session continuation, local/worktree targets, pinned Skill/MCP/Plugin authority, NeedsAttention, retry, stop conditions, restart reconciliation, and typed Event triggers.
- Acceptance: tests must cover schedule preview, overlap, misfire/retry, grant drift, continuation lineage, Event source authentication/dedup/conflict/fan-out, worktree isolation, and no restoration of temporary approvals or leases.
