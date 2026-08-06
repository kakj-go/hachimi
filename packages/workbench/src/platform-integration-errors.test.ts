import { describe, expect, it } from "vitest";

import { integrationFailureMessage } from "./platform-integration-errors";

describe("integrationFailureMessage", () => {
  it("explains duplicate platform identities without exposing database details", () => {
    const error = {
      code: "integration_identity_conflict",
      message:
        "UNIQUE constraint failed: integration_provider_accounts.provider_id, integration_provider_accounts.tenant_key",
    };

    const message = integrationFailureMessage(error, true);

    expect(message).toContain("已连接到另一个账户");
    expect(message).toContain("更新现有账户的凭据");
    expect(message).not.toContain("UNIQUE");
    expect(message).not.toContain("integration_provider_accounts");
  });

  it("keeps unknown actionable failures intact", () => {
    expect(
      integrationFailureMessage({ code: "provider_rejected", message: "App Secret 无效" }, true),
    ).toBe("App Secret 无效");
  });

  it.each([
    ["integration_input_invalid", "账户名称和标识"],
    ["integration_credentials_invalid", "平台凭据"],
    ["integration_capability_required", "至少启用"],
    ["integration_api_unsupported", "不支持企业 API"],
  ])("localizes %s without exposing the backend message", (code, expected) => {
    const message = integrationFailureMessage(
      { code, message: "SQLITE_CONSTRAINT: internal backend detail" },
      true,
    );

    expect(message).toContain(expected);
    expect(message).not.toContain("SQLITE");
  });
});
