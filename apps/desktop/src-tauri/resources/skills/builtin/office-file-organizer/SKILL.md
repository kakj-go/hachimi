---
name: office-file-organizer
description: Inspect, classify, deduplicate, rename, and reorganize files within an explicitly authorized directory. Use for cleanup, filing, naming normalization, project handoff, duplicate review, and folder-structure tasks that require a preview before mutation.
---

# Office file organizer

Operate only through Workspace tools inside the current authorized roots. This Skill never expands those roots and never authorizes deletion, overwrite, or external upload.

1. Inventory with bounded listing and search. Record relative paths, types, sizes, dates when available, and content hashes; do not read unrelated file bodies.
2. Derive categories and naming rules from the user's request. Keep ambiguous, sensitive, hidden, system, and unsupported files in a review group.
3. Produce a deterministic plan before writing: source, destination, action, reason, collision outcome, and rollback mapping.
4. Detect case-only collisions, reserved names, duplicate destinations, extension changes, cross-volume moves, links, and files changed since inventory.
5. Preview counts and examples. Require current Policy/Approval/Sandbox/Host authority for the exact final target set.
6. Execute in bounded batches with optimistic hashes. Never silently overwrite; use a deterministic conflict suffix or stop according to the approved plan.
7. Re-list affected directories, compare hashes and counts, and return a controlled manifest Artifact plus unresolved items.

Deletion is not implied by “organize” or “deduplicate.” Prefer quarantine or a review list unless deletion is explicitly requested and authorized. On partial failure, stop, preserve the manifest, and use the rollback mapping rather than guessing.

Read [validation.md](references/validation.md) before applying a file plan.
