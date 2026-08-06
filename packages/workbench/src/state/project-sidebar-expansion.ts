export const PROJECT_SIDEBAR_EXPANSION_STORAGE_KEY = "hachimi.project-sidebar.expansion.v1";

export interface ProjectSidebarExpansionState {
  projectsExpanded: boolean;
  generalExpanded: boolean;
  expandedProjectIds: string[];
}

const defaultState = (): ProjectSidebarExpansionState => ({
  projectsExpanded: true,
  generalExpanded: true,
  expandedProjectIds: [],
});

export function loadProjectSidebarExpansion(
  storage: Pick<Storage, "getItem"> | undefined = typeof window === "undefined"
    ? undefined
    : window.localStorage,
): ProjectSidebarExpansionState {
  if (!storage) return defaultState();
  try {
    const parsed = JSON.parse(
      storage.getItem(PROJECT_SIDEBAR_EXPANSION_STORAGE_KEY) ?? "null",
    ) as Partial<ProjectSidebarExpansionState> | null;
    if (!parsed || typeof parsed !== "object") return defaultState();
    return {
      projectsExpanded:
        typeof parsed.projectsExpanded === "boolean" ? parsed.projectsExpanded : true,
      generalExpanded: typeof parsed.generalExpanded === "boolean" ? parsed.generalExpanded : true,
      expandedProjectIds: Array.isArray(parsed.expandedProjectIds)
        ? [
            ...new Set(
              parsed.expandedProjectIds.filter((id): id is string => typeof id === "string"),
            ),
          ]
        : [],
    };
  } catch {
    return defaultState();
  }
}

export function persistProjectSidebarExpansion(
  state: ProjectSidebarExpansionState,
  storage: Pick<Storage, "setItem"> | undefined = typeof window === "undefined"
    ? undefined
    : window.localStorage,
) {
  try {
    storage?.setItem(PROJECT_SIDEBAR_EXPANSION_STORAGE_KEY, JSON.stringify(state));
  } catch {
    // A storage quota or privacy setting must not break sidebar navigation.
  }
}
