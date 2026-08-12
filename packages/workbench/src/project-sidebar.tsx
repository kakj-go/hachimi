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
  Folder,
  FolderOpen,
  GitFork,
  MessageCircle,
  MoreHorizontal,
  Pin,
  Play,
  Plus,
  Search,
  SearchField,
  Settings,
  Sidebar,
  Trash2,
} from "@hachimi/ui";
import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import { compareSidebarSessions } from "./project-sidebar-order";
import {
  loadProjectSidebarExpansion,
  persistProjectSidebarExpansion,
} from "./state/project-sidebar-expansion";

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
  motionLabEnabled: boolean;
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
  runningSessionIds: ReadonlySet<string>;
  failedSessionIds: ReadonlySet<string>;
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
  const restoredExpansion = loadProjectSidebarExpansion();
  const [projectsExpanded, setProjectsExpanded] = createSignal(restoredExpansion.projectsExpanded);
  const [generalExpanded, setGeneralExpanded] = createSignal(restoredExpansion.generalExpanded);
  const [expandedProjectIds, setExpandedProjectIds] = createSignal<ReadonlySet<string>>(
    new Set(restoredExpansion.expandedProjectIds),
  );
  const normalizedSearch = createMemo(() => search().trim().toLocaleLowerCase());
  let lastExpandedSessionId: string | undefined;

  const filteredProjects = createMemo(() => {
    const query = normalizedSearch();
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
    const query = normalizedSearch();
    const projectMatches = Boolean(
      query &&
      projectId &&
      props.projects.some(
        (project) =>
          project.id === projectId && project.displayName.toLocaleLowerCase().includes(query),
      ),
    );
    return props.sessions
      .filter(
        (session) =>
          session.archived === showArchived() &&
          sessionProjectId(session) === projectId &&
          (!query || projectMatches || session.title.toLocaleLowerCase().includes(query)),
      )
      .sort(compareSidebarSessions);
  };

  const projectSectionVisible = () => projectsExpanded() || Boolean(normalizedSearch());
  const generalSectionVisible = () => generalExpanded() || Boolean(normalizedSearch());
  const projectSessionsVisible = (projectId: string) =>
    expandedProjectIds().has(projectId) || Boolean(normalizedSearch());

  function toggleProject(projectId: string) {
    setExpandedProjectIds((entries) => {
      const next = new Set(entries);
      if (next.has(projectId)) next.delete(projectId);
      else next.add(projectId);
      return next;
    });
  }

  createEffect(() => {
    const validProjectIds = new Set(props.projects.map((project) => project.id));
    setExpandedProjectIds((entries) => {
      const next = new Set([...entries].filter((id) => validProjectIds.has(id)));
      return next.size === entries.size ? entries : next;
    });
  });

  createEffect(() => {
    const selectedSessionId = props.selectedSessionId;
    if (!selectedSessionId) {
      lastExpandedSessionId = undefined;
      return;
    }
    if (selectedSessionId === lastExpandedSessionId) return;
    lastExpandedSessionId = selectedSessionId;
    const selectedSession = props.sessions.find((session) => session.id === selectedSessionId);
    if (!selectedSession) return;
    const projectId = sessionProjectId(selectedSession);
    if (projectId) {
      setProjectsExpanded(true);
      setExpandedProjectIds((entries) => new Set(entries).add(projectId));
    } else {
      setGeneralExpanded(true);
    }
  });

  createEffect(() => {
    persistProjectSidebarExpansion({
      projectsExpanded: projectsExpanded(),
      generalExpanded: generalExpanded(),
      expandedProjectIds: [...expandedProjectIds()],
    });
  });

  const sessionRow = (session: SessionRecord) => (
    <div class="session-row-shell" classList={{ selected: props.selectedSessionId === session.id }}>
      <Button
        type="button"
        aria-current={props.selectedSessionId === session.id ? "page" : undefined}
        data-testid={`session-select-${session.id}`}
        onClick={() => props.onSelectSession(session)}
      >
        <MessageCircle class="session-kind-icon" size={15} aria-hidden="true" />
        <span class="session-title">{session.title}</span>
        <Show when={session.parentSessionId}>
          <GitFork
            class="session-meta-icon"
            size={12}
            aria-label={i18n.locale() === "zh-CN" ? "Fork 会话" : "Forked session"}
          />
        </Show>
        <Show when={session.pinned}>
          <Pin class="session-meta-icon" size={12} aria-label={i18n.t("workbench.pinned")} />
        </Show>
        <span class="session-state-slot">
          <Show
            when={props.runningSessionIds.has(session.id)}
            fallback={
              <Show when={props.unreadSessionIds.has(session.id)}>
                <i
                  class="session-unread-dot"
                  classList={{ failed: props.failedSessionIds.has(session.id) }}
                  aria-label={props.failedSessionIds.has(session.id) ? "Failed" : "Unread"}
                />
              </Show>
            }
          >
            <i class="session-running-spinner" aria-label="Running" />
          </Show>
        </span>
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
            icon: <Pin size={15} />,
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
        <Show when={props.motionLabEnabled}>
          <Button
            type="button"
            data-testid="motion-lab-open"
            onClick={() => props.openMotionLab()}
          >
            <Play size={17} />
            <span>{i18n.locale() === "zh-CN" ? "动作库实验室" : "Motion Library Lab"}</span>
          </Button>
        </Show>
      </nav>
      <div class="project-sidebar-scroll">
        <section class="project-list-section">
          <div class="project-list-heading">
            <div class="project-list-heading-title">
              <Button
                type="button"
                class="project-section-toggle"
                data-testid="workbench-toggle-projects"
                aria-label={
                  projectsExpanded()
                    ? i18n.locale() === "zh-CN"
                      ? "收起项目"
                      : "Collapse projects"
                    : i18n.locale() === "zh-CN"
                      ? "展开项目"
                      : "Expand projects"
                }
                aria-expanded={projectSectionVisible()}
                onClick={() => setProjectsExpanded((value) => !value)}
              >
                <ChevronDown
                  class="project-section-chevron"
                  classList={{ collapsed: !projectSectionVisible() }}
                  size={14}
                />
              </Button>
              <h2>{i18n.t("workbench.projects")}</h2>
            </div>
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
          <Show when={projectSectionVisible()}>
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
                {(project) => (
                  <>
                    <div class="project-row-shell">
                      <Button
                        type="button"
                        class="project-row"
                        data-testid={`project-select-${project.id}`}
                        aria-current={props.selectedProjectId === project.id ? "page" : undefined}
                        aria-expanded={projectSessionsVisible(project.id)}
                        title={project.displayName}
                        onClick={() => {
                          props.onSelectProject(project.id);
                          toggleProject(project.id);
                        }}
                      >
                        <span class="project-row-main">
                          {projectSessionsVisible(project.id) ? (
                            <FolderOpen size={16} />
                          ) : (
                            <Folder size={16} />
                          )}
                          <span class="project-row-name">{project.displayName}</span>
                        </span>
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
                              icon: <Pin size={16} />,
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
                    <Show when={projectSessionsVisible(project.id)}>
                      <div class="project-sessions">
                        <For each={visibleSessions(project.id)}>{sessionRow}</For>
                        <Show when={visibleSessions(project.id).length === 0}>
                          <p class="session-empty">{i18n.t("workbench.noSessions")}</p>
                        </Show>
                      </div>
                    </Show>
                  </>
                )}
              </For>
            </Show>
          </Show>
        </section>
        <section class="project-list-section">
          <div class="project-list-heading">
            <div class="project-list-heading-title">
              <Button
                type="button"
                class="project-section-toggle"
                data-testid="workbench-toggle-general"
                aria-label={
                  generalExpanded()
                    ? i18n.locale() === "zh-CN"
                      ? "收起通用对话"
                      : "Collapse general chats"
                    : i18n.locale() === "zh-CN"
                      ? "展开通用对话"
                      : "Expand general chats"
                }
                aria-expanded={generalSectionVisible()}
                onClick={() => setGeneralExpanded((value) => !value)}
              >
                <ChevronDown
                  class="project-section-chevron"
                  classList={{ collapsed: !generalSectionVisible() }}
                  size={14}
                />
              </Button>
              <h2>{i18n.locale() === "zh-CN" ? "通用对话" : "General chats"}</h2>
            </div>
          </div>
          <Show when={generalSectionVisible()}>
            <div class="project-sessions general-sessions">
              <For each={visibleSessions(undefined)}>{sessionRow}</For>
              <Show when={visibleSessions(undefined).length === 0}>
                <p class="session-empty">{i18n.locale() === "zh-CN" ? "暂无对话" : "No chats"}</p>
              </Show>
            </div>
          </Show>
        </section>
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
