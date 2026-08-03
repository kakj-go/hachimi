import type { RunRecord, SessionRecord } from "@hachimi/contracts";
import type { ComposerAttachmentPreview } from "../composer-attachments";

export const SELECTED_PROJECT_STORAGE_KEY = "hachimi.workbench.selectedProjectId";
export const SELECTED_SESSION_STORAGE_KEY = "hachimi.workbench.selectedSessionId";
export const PINNED_PROJECTS_STORAGE_KEY = "hachimi.workbench.pinnedProjectIds";
export const REMOVED_PROJECTS_STORAGE_KEY = "hachimi.workbench.removedProjectIds";
export const READ_SESSIONS_STORAGE_KEY = "hachimi.workbench.readTerminalRuns.v2";

export function readSessionSelection(key: string): string | undefined {
  try {
    return window.sessionStorage.getItem(key) ?? undefined;
  } catch {
    return undefined;
  }
}

export function persistSessionSelection(key: string, value: string | undefined) {
  try {
    if (value) window.sessionStorage.setItem(key, value);
    else window.sessionStorage.removeItem(key);
  } catch {
    /* WebView storage can be unavailable during teardown. */
  }
}

export function readLocalJson<T>(key: string, fallback: T): T {
  try {
    const value = window.localStorage.getItem(key);
    return value ? (JSON.parse(value) as T) : fallback;
  } catch {
    return fallback;
  }
}

export function persistLocalJson(key: string, value: unknown) {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* best effort */
  }
}

export function fileSourceKey(file: File): string {
  return [file.webkitRelativePath, file.name, file.size, file.lastModified].join(":");
}

export function createFileAttachmentPreview(file: File): ComposerAttachmentPreview {
  const previewUrl =
    file.type.startsWith("image/") && typeof URL.createObjectURL === "function"
      ? URL.createObjectURL(file)
      : undefined;
  return {
    id: crypto.randomUUID(),
    sourceKey: fileSourceKey(file),
    kind: "file",
    name: file.name,
    mimeType: file.type || "application/octet-stream",
    byteSize: file.size,
    fileCount: 1,
    ...(previewUrl ? { previewUrl } : {}),
  };
}

export function revokeAttachmentPreview(attachment: ComposerAttachmentPreview) {
  if (attachment.previewUrl && typeof URL.revokeObjectURL === "function") {
    URL.revokeObjectURL(attachment.previewUrl);
  }
}

export function sessionProjectId(session: SessionRecord): string | undefined {
  return session.context.kind === "project" ? session.context.project_id : undefined;
}

export function isTerminalRunStatus(status: RunRecord["status"]): boolean {
  return ["succeeded", "failed", "timed_out", "cancelled", "interrupted", "lost"].includes(status);
}
