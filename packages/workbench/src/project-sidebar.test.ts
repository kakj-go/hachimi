import type { SessionRecord } from "@hachimi/contracts";
import { describe, expect, it } from "vitest";
import { compareSidebarSessions } from "./project-sidebar-order";

function session(
  id: string,
  createdAtMs: number,
  options: { pinned?: boolean; updatedAtMs?: number } = {},
): SessionRecord {
  return {
    id,
    context: { kind: "workspace", workspace_id: `workspace-${id}` },
    entryProfile: "workbench",
    title: id,
    archived: false,
    pinned: options.pinned ?? false,
    parentSessionId: null,
    sourceRunId: null,
    createdAtMs,
    updatedAtMs: options.updatedAtMs ?? createdAtMs,
  };
}

describe("compareSidebarSessions", () => {
  it("keeps pinned sessions first and sorts each group by creation time", () => {
    const sessions = [
      session("older", 100, { updatedAtMs: 900 }),
      session("newest", 300, { updatedAtMs: 300 }),
      session("pinned-older", 50, { pinned: true }),
      session("middle", 200, { updatedAtMs: 1_000 }),
      session("pinned-newer", 150, { pinned: true }),
    ];

    expect(sessions.toSorted(compareSidebarSessions).map(({ id }) => id)).toEqual([
      "pinned-newer",
      "pinned-older",
      "newest",
      "middle",
      "older",
    ]);
  });

  it("uses the id as a deterministic tie breaker", () => {
    const sessions = [session("session-b", 100), session("session-a", 100)];

    expect(sessions.toSorted(compareSidebarSessions).map(({ id }) => id)).toEqual([
      "session-a",
      "session-b",
    ]);
  });

  it("does not change visual order when the selected session is moved to the source array front", () => {
    const newest = session("newest", 300);
    const middle = session("middle", 200);
    const oldest = session("oldest", 100);
    const afterSelection = [oldest, newest, middle];

    expect(afterSelection.toSorted(compareSidebarSessions).map(({ id }) => id)).toEqual([
      "newest",
      "middle",
      "oldest",
    ]);
  });
});
