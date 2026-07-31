import type { WorkbenchRoute } from "@hachimi/contracts";

export const WORKBENCH_ROUTES: readonly WorkbenchRoute[] = [
  "home",
  "desktop-control",
  "settings/general",
  "settings/appearance",
  "settings/llm",
  "settings/avatar",
  "settings/motion",
  "settings/voice",
  "settings/skills",
  "settings/mcp",
  "settings/local-hosts",
  "developer/motion-lab",
];

export const SETTINGS_ROUTES = WORKBENCH_ROUTES.filter(
  (route): route is Extract<WorkbenchRoute, `settings/${string}`> => route.startsWith("settings/"),
);

export function normalizeWorkbenchRoute(value: string | null | undefined): WorkbenchRoute {
  if (WORKBENCH_ROUTES.includes(value as WorkbenchRoute)) return value as WorkbenchRoute;
  return value?.startsWith("settings/") ? "settings/general" : "home";
}

export function pushWorkbenchHistory(
  history: readonly WorkbenchRoute[],
  index: number,
  route: WorkbenchRoute,
): { history: WorkbenchRoute[]; index: number } {
  if (history[index] === route) return { history: [...history], index };
  const next = [...history.slice(0, index + 1), route];
  return { history: next, index: next.length - 1 };
}
