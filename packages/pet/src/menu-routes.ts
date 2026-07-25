import type { WorkbenchRoute } from "@hachimi/contracts";

export type WorkbenchMenuId = "workbench" | "llm" | "avatar" | "voice" | "interaction";

export const WORKBENCH_MENU_ROUTES: Readonly<Record<WorkbenchMenuId, WorkbenchRoute>> = {
  workbench: "home",
  llm: "settings/llm",
  avatar: "settings/avatar",
  voice: "settings/voice",
  interaction: "settings/motion",
};
