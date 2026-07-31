import type { ProjectRecord, SessionRecord } from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  Archive,
  Button,
  CalendarClock,
  Check,
  ChevronDown,
  Dropdown,
  ExternalLink,
  FolderOpen,
  GitBranch,
  GitFork,
  MessageCircle,
  Monitor,
  MoreHorizontal,
  Play,
  Plus,
  Search,
  SearchField,
  Settings,
  Sidebar,
  Trash2,
} from "@hachimi/ui";
import { For, Show, createEffect, createMemo, createSignal } from "solid-js";

export type ProjectMenuAction =
  | "pin"
  | "open"
  | "create_permanent_worktree"
  | "rename"
  | "mark_read"
  | "archive_tasks"
  | "remove";

export type SessionMenuAction = "pin" | "rename" | "fork" | "archive" | "unarchive";

export function ProjectSidebar(props: {
  openSettings: () => void;
  openMotionLab: () => void;
  openDesktopControl: () => void;
  motionLabEnabled: boolean;
  desktopControlEnabled: boolean;
  schedulerEnabled: boolean;
  onNewTask: (projectId?: string) => void;
  onOpenTasks: () => void;
  activeView: "agent" | "tasks";
  projects: ProjectRecord[];
  sessions: SessionRecord[];
  selectedProjectId: string | undefined;
  selectedSessionId: string | undefined;
  pinnedProjectIds: readonly string[];
  unreadSessionIds: ReadonlySet<string>;
  loading: boolean;
  addingProject: boolean;
  onAddProject: () => void;
  onSelectProject: (projectId: string) => void;
  onSelectSession: (session: SessionRecord) => void;
  onProjectAction: (project: ProjectRecord, action: ProjectMenuAction) => void;
  onSessionAction: (session: SessionRecord, action: SessionMenuAction) => void;
}) {
  const i18n = useI18n();
  const [search, setSearch] = createSignal("");
  const [searchOpen, setSearchOpen] = createSignal(false);
  const [showArchived, setShowArchived] = createSignal(false);
  const [expandedProjectIds, setExpandedProjectIds] = createSignal<string[]>([]);

  createEffect(() => {
    const projectIds = props.projects.map((project) => project.id);
    setExpandedProjectIds((current) => [
      ...current.filter((id) => projectIds.includes(id)),
      ...projectIds.filter((id) => !current.includes(id)),
    ]);
  });

  const filteredProjects = createMemo(() => {
    const query = search().trim().toLocaleLowerCase();
    if (!query) return props.projects;
    return props.projects.filter(
      (project) =>
        project.displayName.toLocaleLowerCase().includes(query) ||
        props.sessions.some(
          (session) =>
            sessionProjectId(session) === project.id &&
            session.title.toLocaleLowerCase().includes(query),
        ),
    );
  });

  const visibleSessions = (projectId?: string) => {
    const query = search().trim().toLocaleLowerCase();
    return props.sessions
      .filter(
        (session) =>
          session.archived === showArchived() &&
          sessionProjectId(session) === projectId &&
          (!query || session.title.toLocaleLowerCase().includes(query)),
      )
      .sort((left, right) => Number(right.pinned) - Number(left.pinned));
  };

  const toggleProject = (projectId: string) => {
    props.onSelectProject(projectId);
    setExpandedProjectIds((current) =>
      current.includes(projectId)
        ? current.filter((id) => id !== projectId)
        : [...current, projectId],
    );
  };

  const sessionRow = (session: SessionRecord) => (
    <div class="session-row-shell">
      <Button
        type="button"
        classList={{ selected: props.selectedSessionId === session.id }}
        data-testid={`session-select-${session.id}`}
        onClick={() => props.onSelectSession(session)}
      >
        <Show when={session.parentSessionId} fallback={<MessageCircle size={14} />}>
          <GitFork size={14} />
        </Show>
        <span>{session.title}</span>
        <Show when={session.pinned}>
          <GitBranch size={12} aria-label={i18n.t("workbench.pinned")} />
        </Show>
        <Show when={props.unreadSessionIds.has(session.id)}>
          <i class="session-unread-dot" aria-label="Unread" />
        </Show>
      </Button>
      <Dropdown
        label={
          i18n.locale() === "zh-CN"
            ? `${session.title} 会话操作`
            : `${session.title} session actions`
        }
        triggerTestId={`session-more-${session.id}`}
        actions={[
          {
            id: "pin",
            label: session.pinned
              ? i18n.locale() === "zh-CN"
                ? "取消置顶"
                : "Unpin"
              : i18n.locale() === "zh-CN"
                ? "置顶"
                : "Pin",
            icon: <GitBranch size={15} />,
          },
          {
            id: "rename",
            label: i18n.locale() === "zh-CN" ? "重命名" : "Rename",
            icon: <MoreHorizontal size={15} />,
          },
          {
            id: "fork",
            label: i18n.locale() === "zh-CN" ? "从最新终态运行 Fork" : "Fork latest terminal run",
            icon: <GitFork size={15} />,
          },
          {
            id: session.archived ? "unarchive" : "archive",
            label: session.archived
              ? i18n.locale() === "zh-CN"
                ? "恢复"
                : "Restore"
              : i18n.locale() === "zh-CN"
                ? "归档"
                : "Archive",
            icon: <Archive size={15} />,
            separatorBefore: true,
          },
        ]}
        onSelect={(action) => props.onSessionAction(session, action as SessionMenuAction)}
      >
        <MoreHorizontal size={15} />
      </Dropdown>
    </div>
  );

  return (
    <Sidebar class="project-sidebar">
      <div class="project-sidebar-brand">
        <span class="hachimi-mini-mark">H</span>
        <strong>Hachimi</strong>
        <Button
          type="button"
          aria-label={i18n.t("settings.search")}
          aria-expanded={searchOpen()}
          onClick={() => setSearchOpen((value) => !value)}
        >
          <Search size={17} />
        </Button>
      </div>
      <Show when={searchOpen()}>
        <SearchField
          label={i18n.t("settings.search")}
          placeholder={i18n.t("settings.search")}
          value={search()}
          onInput={(event) => setSearch(event.currentTarget.value)}
        />
      </Show>
      <nav class="project-quick-nav" aria-label={i18n.t("workbench.home")}>
        <Button
          type="button"
          classList={{ active: props.activeView === "agent" && !showArchived() }}
          data-testid="workbench-new-task"
          onClick={() => {
            setShowArchived(false);
            props.onNewTask();
          }}
        >
          <Plus size={17} /> <span>{i18n.t("workbench.newTask")}</span>
        </Button>
        <Button
          type="button"
          classList={{ active: showArchived() }}
          data-testid="workbench-archived-sessions"
          onClick={() => setShowArchived((value) => !value)}
        >
          <Archive size={17} />
          <span>{i18n.locale() === "zh-CN" ? "已归档" : "Archived"}</span>
        </Button>
        <Show when={props.schedulerEnabled}>
          <Button
            type="button"
            classList={{ active: props.activeView === "tasks" }}
            data-testid="workbench-task-tab"
            onClick={() => props.onOpenTasks()}
          >
            <CalendarClock size={17} />
            <span>{i18n.locale() === "zh-CN" ? "任务" : "Tasks"}</span>
          </Button>
        </Show>
        <Show when={props.desktopControlEnabled}>
          <Button
            type="button"
            data-testid="workbench-desktop-control"
            onClick={props.openDesktopControl}
          >
            <Monitor size={17} />
            <span>{i18n.locale() === "zh-CN" ? "桌面控制" : "Desktop Control"}</span>
          </Button>
        </Show>
        <Show when={props.motionLabEnabled}>
          <Button type="button" onClick={() => props.openMotionLab()}>
            <Play size={17} />
            <span>{i18n.locale() === "zh-CN" ? "动作库实验室" : "Motion Library Lab"}</span>
          </Button>
        </Show>
      </nav>
      <div class="project-sidebar-scroll">
        <section class="project-list-section">
          <div class="project-list-heading">
            <h2>{i18n.t("workbench.projects")}</h2>
            <Button
              type="button"
              data-testid="workbench-add-project"
              aria-label={i18n.t("workbench.addProject")}
              title={i18n.t("workbench.addProject")}
              disabled={props.addingProject}
              onClick={() => props.onAddProject()}
            >
              <Plus size={14} />
            </Button>
          </div>
          <Show
            when={filteredProjects().length > 0}
            fallback={
              <p class="project-empty">
                {props.loading
                  ? i18n.t("workbench.loadingProjects")
                  : i18n.t("workbench.noProjects")}
              </p>
            }
          >
            <For each={filteredProjects()}>
              {(project) => {
                const expanded = () => expandedProjectIds().includes(project.id);
                return (
                  <>
                    <div class="project-row-shell">
                      <Button
                        type="button"
                        class="project-row"
                        classList={{ selected: props.selectedProjectId === project.id }}
                        aria-expanded={expanded()}
                        title={project.displayName}
                        onClick={() => toggleProject(project.id)}
                      >
                        <span class="project-row-main">
                          <FolderOpen size={16} />
                          <span class="project-row-name">{project.displayName}</span>
                        </span>
                        <ChevronDown
                          size={15}
                          class="project-chevron"
                          classList={{ collapsed: !expanded() }}
                        />
                      </Button>
                      <div class="project-row-actions">
                        <Dropdown
                          label={`${project.displayName} project actions`}
                          triggerTestId={`project-more-${project.id}`}
                          actions={[
                            {
                              id: "pin",
                              label: props.pinnedProjectIds.includes(project.id)
                                ? i18n.locale() === "zh-CN"
                                  ? "取消置顶项目"
                                  : "Unpin project"
                                : i18n.locale() === "zh-CN"
                                  ? "置顶项目"
                                  : "Pin project",
                              icon: <GitBranch size={16} />,
                            },
                            {
                              id: "open",
                              label:
                                i18n.locale() === "zh-CN"
                                  ? "在资源管理器中打开"
                                  : "Open in file explorer",
                              icon: <ExternalLink size={16} />,
                            },
                            {
                              id: "create_permanent_worktree",
                              label:
                                i18n.locale() === "zh-CN"
                                  ? "创建永久工作树"
                                  : "Create permanent worktree",
                              icon: <GitFork size={16} />,
                              disabled: !project.gitRoot,
                            },
                            {
                              id: "rename",
                              label: i18n.locale() === "zh-CN" ? "重命名项目" : "Rename project",
                              icon: <MoreHorizontal size={16} />,
                            },
                            {
                              id: "mark_read",
                              label: i18n.locale() === "zh-CN" ? "全部标为已读" : "Mark all read",
                              icon: <Check size={16} />,
                            },
                            {
                              id: "archive_tasks",
                              label: i18n.locale() === "zh-CN" ? "归档任务" : "Archive tasks",
                              icon: <Archive size={16} />,
                              disabled: visibleSessions(project.id).length === 0,
                            },
                            {
                              id: "remove",
                              label: i18n.locale() === "zh-CN" ? "移除" : "Remove",
                              icon: <Trash2 size={16} />,
                              danger: true,
                              separatorBefore: true,
                            },
                          ]}
                          onSelect={(action) =>
                            props.onProjectAction(project, action as ProjectMenuAction)
                          }
                        >
                          <MoreHorizontal size={16} />
                        </Dropdown>
                        <Button
                          type="button"
                          class="project-new-task"
                          data-testid={`project-new-task-${project.id}`}
                          aria-label={`New task in ${project.displayName}`}
                          onClick={() => props.onNewTask(project.id)}
                        >
                          <Plus size={16} />
                        </Button>
                      </div>
                    </div>
                    <Show when={expanded()}>
                      <div class="project-sessions">
                        <For each={visibleSessions(project.id)}>{sessionRow}</For>
                        <Show when={visibleSessions(project.id).length === 0}>
                          <p class="session-empty">{i18n.t("workbench.noSessions")}</p>
                        </Show>
                      </div>
                    </Show>
                  </>
                );
              }}
            </For>
          </Show>
        </section>
        <Show when={visibleSessions(undefined).length > 0}>
          <section class="project-list-section">
            <div class="project-list-heading">
              <h2>{i18n.locale() === "zh-CN" ? "通用会话" : "General sessions"}</h2>
            </div>
            <div class="project-sessions general-sessions">
              <For each={visibleSessions(undefined)}>{sessionRow}</For>
            </div>
          </section>
        </Show>
      </div>
      <Button
        type="button"
        class="sidebar-account"
        data-testid="workbench-open-settings"
        aria-label={i18n.t("settings.title")}
        onClick={() => props.openSettings()}
      >
        <span class="account-avatar">M</span>
        <span>
          <strong>my_codex</strong>
          <small>{i18n.t("settings.title")}</small>
        </span>
        <Settings size={17} />
      </Button>
    </Sidebar>
  );
}

function sessionProjectId(session: SessionRecord): string | undefined {
  return session.context.kind === "project" ? session.context.project_id : undefined;
}
