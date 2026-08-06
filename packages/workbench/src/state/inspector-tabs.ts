import type { InspectorResource } from "./workbench-layout";

export type InspectorTab = {
  id: string;
  resource: InspectorResource;
};

export type InspectorTabsState = {
  tabs: InspectorTab[];
  activeTabId: string | undefined;
  resource: InspectorResource;
};

export const EMPTY_INSPECTOR_TABS: InspectorTabsState = {
  tabs: [],
  activeTabId: undefined,
  resource: { kind: "tools" },
};

function resourceKey(resource: InspectorResource) {
  switch (resource.kind) {
    case "tools":
      return "tools";
    case "review":
      return "review";
    case "files":
      return "files";
    case "browser":
      return [
        "browser",
        resource.surface ?? "embedded",
        resource.leaseId ?? resource.browserSessionId ?? "workspace",
      ].join(":");
    case "computer":
      return `computer:${resource.controlSessionId ?? "current"}`;
    case "sources":
      return "sources";
    case "plan":
      return `plan:${resource.planId}`;
    case "attachment":
      return `attachment:${resource.attachmentId}`;
    case "artifact":
      return `artifact:${resource.artifactId}`;
  }
}

export function showInspectorTabs(state: InspectorTabsState): InspectorTabsState {
  if (state.tabs.length === 0) return EMPTY_INSPECTOR_TABS;
  const active = state.tabs.find((tab) => tab.id === state.activeTabId) ?? state.tabs.at(-1)!;
  return { ...state, activeTabId: active.id, resource: active.resource };
}

export function showInspectorLauncher(state: InspectorTabsState): InspectorTabsState {
  return { ...state, resource: { kind: "tools" } };
}

export function openInspectorTab(
  state: InspectorTabsState,
  resource: InspectorResource,
  createId: () => string,
): InspectorTabsState {
  if (resource.kind === "tools") return showInspectorLauncher(state);

  const key = resourceKey(resource);
  const existing = state.tabs.find((tab) => resourceKey(tab.resource) === key);
  if (existing) {
    const tabs = state.tabs.map((tab) => (tab.id === existing.id ? { ...tab, resource } : tab));
    return { tabs, activeTabId: existing.id, resource };
  }

  const tab = { id: createId(), resource };
  return { tabs: [...state.tabs, tab], activeTabId: tab.id, resource };
}

export function selectInspectorTab(state: InspectorTabsState, tabId: string): InspectorTabsState {
  const tab = state.tabs.find((candidate) => candidate.id === tabId);
  return tab ? { ...state, activeTabId: tab.id, resource: tab.resource } : state;
}

export function closeInspectorTab(state: InspectorTabsState, tabId: string): InspectorTabsState {
  const index = state.tabs.findIndex((tab) => tab.id === tabId);
  if (index < 0) return state;

  const tabs = state.tabs.filter((tab) => tab.id !== tabId);
  if (tabs.length === 0) return EMPTY_INSPECTOR_TABS;
  if (state.activeTabId !== tabId) return { ...state, tabs };

  const active = tabs[Math.min(index, tabs.length - 1)]!;
  return {
    tabs,
    activeTabId: active.id,
    resource: state.resource.kind === "tools" ? state.resource : active.resource,
  };
}

export function inspectorTabLabel(resource: InspectorResource, locale: "zh-CN" | "en-US") {
  const zh = locale === "zh-CN";
  switch (resource.kind) {
    case "tools":
      return zh ? "新建" : "New";
    case "review":
      return zh ? "审阅" : "Review";
    case "computer":
      return zh ? "电脑" : "Computer";
    case "files": {
      const name = resource.path?.split(/[\\/]/).filter(Boolean).at(-1);
      return name || (zh ? "文件" : "Files");
    }
    case "plan":
      return zh ? "计划" : "Plan";
    case "browser":
      return zh ? "浏览器" : "Browser";
    case "sources":
      return zh ? "来源" : "Sources";
    case "attachment":
    case "artifact":
      return resource.name;
  }
}
