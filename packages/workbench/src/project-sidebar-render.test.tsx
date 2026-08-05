import type { ProjectRecord, SessionRecord } from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import { type JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ProjectSidebar } from "./project-sidebar";
import { PROJECT_SIDEBAR_EXPANSION_STORAGE_KEY } from "./state/project-sidebar-expansion";

function className(baseClass?: string, classList?: Record<string, boolean | undefined>) {
  return [
    baseClass,
    ...Object.entries(classList ?? {})
      .filter(([, enabled]) => enabled)
      .map(([name]) => name),
  ]
    .filter(Boolean)
    .join(" ");
}

function icon(name: string) {
  return (props: { class?: string; classList?: Record<string, boolean | undefined> }) => (
    <i data-icon={name} class={className(props.class, props.classList)} />
  );
}

vi.mock("@hachimi/ui", () => ({
  Archive: icon("Archive"),
  CalendarClock: icon("CalendarClock"),
  Check: icon("Check"),
  ChevronDown: icon("ChevronDown"),
  ExternalLink: icon("ExternalLink"),
  Folder: icon("Folder"),
  FolderOpen: icon("FolderOpen"),
  GitFork: icon("GitFork"),
  MessageCircle: icon("MessageCircle"),
  MoreHorizontal: icon("MoreHorizontal"),
  Pin: icon("Pin"),
  Play: icon("Play"),
  Plus: icon("Plus"),
  Search: icon("Search"),
  Settings: icon("Settings"),
  Trash2: icon("Trash2"),
  Button: (props: {
    children?: JSX.Element;
    class?: string;
    classList?: Record<string, boolean | undefined>;
    disabled?: boolean;
    title?: string;
    "aria-label"?: string;
    "aria-current"?: boolean | "true" | "false" | "step" | "page" | "location" | "date" | "time";
    "aria-expanded"?: boolean;
    "data-testid"?: string;
    onClick?: () => void;
  }) => (
    <button
      class={className(props.class, props.classList)}
      disabled={props.disabled}
      title={props.title}
      aria-label={props["aria-label"]}
      aria-current={props["aria-current"]}
      aria-expanded={props["aria-expanded"]}
      data-testid={props["data-testid"]}
      onClick={() => props.onClick?.()}
    >
      {props.children}
    </button>
  ),
  Dropdown: (props: { children?: JSX.Element; triggerTestId?: string }) => (
    <button data-testid={props.triggerTestId}>{props.children}</button>
  ),
  SearchField: (props: {
    label: string;
    placeholder?: string;
    value: string;
    onInput?: (event: InputEvent & { currentTarget: HTMLInputElement }) => void;
  }) => (
    <input
      aria-label={props.label}
      placeholder={props.placeholder}
      value={props.value}
      onInput={(event) => props.onInput?.(event)}
    />
  ),
  Sidebar: (props: { class?: string; children?: JSX.Element }) => (
    <aside class={props.class}>{props.children}</aside>
  ),
}));

const project: ProjectRecord = {
  id: "project-1",
  displayName: "hachimi-code",
  rootPath: "D:\\workspace\\hachimi-code",
  gitRoot: "D:\\workspace\\hachimi-code",
  trusted: true,
  createdAtMs: 1,
  updatedAtMs: 1,
};

const projectSession: SessionRecord = {
  id: "project-session",
  context: { kind: "project", project_id: project.id, checkout_id: "checkout-1" },
  entryProfile: "workbench",
  title: "Project conversation",
  archived: false,
  pinned: false,
  parentSessionId: null,
  sourceRunId: null,
  createdAtMs: 2,
  updatedAtMs: 2,
};

function mount(options: { selectedSessionId?: string; sessions?: SessionRecord[] } = {}) {
  const host = document.createElement("div");
  document.body.append(host);
  const onSelectProject = vi.fn();
  const dispose = render(
    () => (
      <I18nProvider initialLocale="zh-CN">
        <ProjectSidebar
          openSettings={vi.fn()}
          openMotionLab={vi.fn()}
          motionLabEnabled={false}
          schedulerEnabled={false}
          onNewTask={vi.fn()}
          onOpenTasks={vi.fn()}
          activeView="agent"
          projects={[project]}
          sessions={options.sessions ?? [projectSession]}
          selectedProjectId={undefined}
          selectedSessionId={options.selectedSessionId}
          pinnedProjectIds={[]}
          unreadSessionIds={new Set()}
          runningSessionIds={new Set()}
          failedSessionIds={new Set()}
          loading={false}
          addingProject={false}
          onAddProject={vi.fn()}
          onSelectProject={onSelectProject}
          onSelectSession={vi.fn()}
          onProjectAction={vi.fn()}
          onSessionAction={vi.fn()}
        />
      </I18nProvider>
    ),
    host,
  );
  return { host, dispose, onSelectProject };
}

beforeEach(() => window.localStorage.clear());

afterEach(() => {
  document.body.replaceChildren();
  vi.clearAllMocks();
});

describe("ProjectSidebar expansion", () => {
  it("always renders an empty General chats section", () => {
    const mounted = mount({ sessions: [] });
    expect(mounted.host.textContent).toContain("通用对话");
    expect(mounted.host.textContent).toContain("暂无对话");
    mounted.dispose();
  });

  it("toggles project conversations and folder state without clearing selection", async () => {
    const mounted = mount();
    const row = mounted.host.querySelector<HTMLButtonElement>('button[title="hachimi-code"]')!;

    expect(row.getAttribute("aria-expanded")).toBe("false");
    expect(mounted.host.textContent).not.toContain("Project conversation");
    expect(row.querySelector('[data-icon="Folder"]')).not.toBeNull();

    row.click();
    expect(mounted.onSelectProject).toHaveBeenCalledWith(project.id);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    expect(mounted.host.textContent).toContain("Project conversation");
    expect(row.querySelector('[data-icon="FolderOpen"]')).not.toBeNull();

    row.click();
    expect(row.getAttribute("aria-expanded")).toBe("false");
    expect(mounted.host.textContent).not.toContain("Project conversation");
    await vi.waitFor(() => {
      const stored = JSON.parse(
        window.localStorage.getItem(PROJECT_SIDEBAR_EXPANSION_STORAGE_KEY) ?? "{}",
      );
      expect(stored.expandedProjectIds).toEqual([]);
    });
    mounted.dispose();
  });

  it("restores persisted expansion and expands the active session project", async () => {
    window.localStorage.setItem(
      PROJECT_SIDEBAR_EXPANSION_STORAGE_KEY,
      JSON.stringify({
        projectsExpanded: false,
        generalExpanded: false,
        expandedProjectIds: [],
      }),
    );
    const mounted = mount({ selectedSessionId: projectSession.id });

    await vi.waitFor(() => expect(mounted.host.textContent).toContain("Project conversation"));
    expect(
      mounted.host
        .querySelector('[data-testid="workbench-toggle-projects"]')
        ?.getAttribute("aria-expanded"),
    ).toBe("true");
    mounted.dispose();
  });

  it("collapses and restores the project and General chats sections independently", () => {
    const mounted = mount({ sessions: [] });
    const projects = mounted.host.querySelector<HTMLButtonElement>(
      '[data-testid="workbench-toggle-projects"]',
    )!;
    const general = mounted.host.querySelector<HTMLButtonElement>(
      '[data-testid="workbench-toggle-general"]',
    )!;

    projects.click();
    expect(mounted.host.querySelector('button[title="hachimi-code"]')).toBeNull();
    expect(general.getAttribute("aria-expanded")).toBe("true");

    general.click();
    expect(mounted.host.textContent).not.toContain("暂无对话");
    projects.click();
    expect(mounted.host.querySelector('button[title="hachimi-code"]')).not.toBeNull();
    mounted.dispose();
  });
});
