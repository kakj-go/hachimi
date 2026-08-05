import { describe, expect, it, vi } from "vitest";

import { mcpHealthMessage, runtimeErrorMessage } from "./runtime-health";

vi.mock("@hachimi/ui", () => ({
  Button: () => null,
  RefreshCw: () => null,
  StatusBanner: () => null,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

describe("runtime health copy", () => {
  it("turns stable runtime codes into actionable Chinese reasons", () => {
    expect(runtimeErrorMessage("gateway_ready_timeout", true)).toBe(
      "本地消息服务启动超时，正在重试。",
    );
    expect(runtimeErrorMessage("computer_protected_desktop", true)).toContain("受保护桌面");
  });

  it("does not expose unknown internal error codes", () => {
    expect(runtimeErrorMessage("sqlx_row_decode_failed", true)).not.toContain("sqlx");
    expect(runtimeErrorMessage("sqlx_row_decode_failed", false)).not.toContain("sqlx");
  });

  it("describes common MCP failures without raw transport details", () => {
    expect(mcpHealthMessage("spawn_failed", true)).toBe("程序不存在或无法启动");
    expect(mcpHealthMessage("mcp_credential_unavailable", true)).toBe("凭据不可用");
    expect(mcpHealthMessage("timeout", true)).toBe("连接超时");
  });
});
