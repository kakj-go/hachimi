import type { SessionRecord } from "@hachimi/contracts";

export function compareSidebarSessions(left: SessionRecord, right: SessionRecord): number {
  return (
    Number(right.pinned) - Number(left.pinned) ||
    right.createdAtMs - left.createdAtMs ||
    left.id.localeCompare(right.id)
  );
}
