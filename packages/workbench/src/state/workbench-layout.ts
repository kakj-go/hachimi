export type InspectorResource =
  | { kind: "tools" }
  | {
      kind: "review";
      path?: string;
      diffRunId?: string;
      diffBaseBranch?: string;
      diffBranches?: string[];
      diffScope?: "run" | "checkout" | "branch" | "session";
    }
  | { kind: "files"; path?: string }
  | { kind: "plan"; planId: string }
  | { kind: "browser"; browserTabId?: string; initialUrl?: string }
  | { kind: "sources" }
  | { kind: "attachment"; attachmentId: string; name: string }
  | { kind: "artifact"; artifactId: string; name: string };

export const WORKBENCH_LAYOUT_STORAGE_KEY = "hachimi.workbench.layout.v2";

export type PersistedWorkbenchLayout = {
  summaryPinned: boolean;
  bottomPanelOpen: boolean;
  sidebarVisible: boolean;
  projectSidebarWidth: number;
  inspectorWidth: number;
  bottomPanelHeight: number;
};

export const DEFAULT_WORKBENCH_LAYOUT: PersistedWorkbenchLayout = {
  summaryPinned: false,
  bottomPanelOpen: false,
  sidebarVisible: false,
  projectSidebarWidth: 288,
  inspectorWidth: 380,
  bottomPanelHeight: 250,
};

const LAYOUT_BOUNDS = {
  projectSidebarWidth: { minimum: 220, maximum: 480 },
  inspectorWidth: { minimum: 300, maximum: 820 },
  bottomPanelHeight: { minimum: 140, maximum: 520 },
} as const;

function boundedLayoutNumber(
  value: unknown,
  fallback: number,
  bounds: { minimum: number; maximum: number },
) {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(bounds.maximum, Math.max(bounds.minimum, Math.round(value)));
}

function normalizedLayout(value: unknown): PersistedWorkbenchLayout {
  if (!value || typeof value !== "object") return { ...DEFAULT_WORKBENCH_LAYOUT };
  const candidate = value as Partial<Record<keyof PersistedWorkbenchLayout, unknown>>;
  return {
    summaryPinned:
      typeof candidate.summaryPinned === "boolean"
        ? candidate.summaryPinned
        : DEFAULT_WORKBENCH_LAYOUT.summaryPinned,
    bottomPanelOpen:
      typeof candidate.bottomPanelOpen === "boolean"
        ? candidate.bottomPanelOpen
        : DEFAULT_WORKBENCH_LAYOUT.bottomPanelOpen,
    sidebarVisible:
      typeof candidate.sidebarVisible === "boolean"
        ? candidate.sidebarVisible
        : DEFAULT_WORKBENCH_LAYOUT.sidebarVisible,
    projectSidebarWidth: boundedLayoutNumber(
      candidate.projectSidebarWidth,
      DEFAULT_WORKBENCH_LAYOUT.projectSidebarWidth,
      LAYOUT_BOUNDS.projectSidebarWidth,
    ),
    inspectorWidth: boundedLayoutNumber(
      candidate.inspectorWidth,
      DEFAULT_WORKBENCH_LAYOUT.inspectorWidth,
      LAYOUT_BOUNDS.inspectorWidth,
    ),
    bottomPanelHeight: boundedLayoutNumber(
      candidate.bottomPanelHeight,
      DEFAULT_WORKBENCH_LAYOUT.bottomPanelHeight,
      LAYOUT_BOUNDS.bottomPanelHeight,
    ),
  };
}

export function inspectorNeedsProjectTools(resource: InspectorResource | undefined) {
  return (
    resource?.kind === "tools" ||
    resource?.kind === "review" ||
    resource?.kind === "files" ||
    resource?.kind === "browser" ||
    resource?.kind === "sources"
  );
}

export function readWorkbenchLayout(): PersistedWorkbenchLayout {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(WORKBENCH_LAYOUT_STORAGE_KEY) ?? "null");
    return normalizedLayout(parsed);
  } catch {
    return { ...DEFAULT_WORKBENCH_LAYOUT };
  }
}

export function persistWorkbenchLayout(layout: PersistedWorkbenchLayout) {
  try {
    window.localStorage.setItem(WORKBENCH_LAYOUT_STORAGE_KEY, JSON.stringify(layout));
  } catch {
    // Layout preferences are best effort.
  }
}
