export function resolveLocalMarkdownPath(target: string, workspaceRoot?: string) {
  let value = target.trim().replace(/^<|>$/g, "");
  try {
    value = decodeURIComponent(value);
  } catch {
    // Keep malformed percent sequences visible but non-interactive.
  }
  value = value.replace(/#L\d+(?:C\d+)?$/i, "").replace(/:\d+(?::\d+)?$/, "");
  if (/^(?:https?|mailto|data|javascript):/i.test(value)) return undefined;
  if (/^file:\/\//i.test(value)) {
    try {
      value = new URL(value).pathname.replace(/^\/(?=[A-Za-z]:\/)/, "");
    } catch {
      return undefined;
    }
  }
  const normalized = value.replaceAll("\\", "/");
  const root = workspaceRoot?.replaceAll("\\", "/").replace(/\/$/, "");
  const absolute = /^[A-Za-z]:\//.test(normalized) || normalized.startsWith("/");
  if (absolute) {
    if (!root) return undefined;
    const prefix = root.toLocaleLowerCase() + "/";
    if (!normalized.toLocaleLowerCase().startsWith(prefix)) return undefined;
    value = normalized.slice(prefix.length);
  } else {
    value = normalized.replace(/^\.\//, "");
  }
  if (!value || value === ".." || value.startsWith("../") || value.includes("/../")) {
    return undefined;
  }
  return value;
}
