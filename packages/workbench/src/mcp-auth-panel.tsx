import type { McpAuthStatusRecord } from "@hachimi/contracts";
import { Button, StatusBanner } from "@hachimi/ui";
import { Show } from "solid-js";

export function McpAuthPanel(props: {
  status: McpAuthStatusRecord | undefined;
  loading: boolean;
  connectorEnabled: boolean;
  copy: (zh: string, en: string) => string;
  onLogin: (scopes: string[]) => void;
  onLogout: () => void;
}) {
  const description = () => {
    switch (props.status?.status) {
      case "oauth":
        return props.copy(
          "已通过 OAuth 登录。访问令牌仅保存在系统安全凭据库中，并在调用前自动刷新。",
          "Signed in with OAuth. Tokens stay in the OS keyring and refresh before use.",
        );
      case "bearer_token":
        return props.copy(
          "当前使用手动 Authorization Header。它与 OAuth 登录互斥。",
          "This server uses a manual Authorization header, which is mutually exclusive with OAuth.",
        );
      case "not_logged_in":
        return props.copy(
          "服务支持 OAuth。登录会在系统浏览器中完成，前端不会接触令牌。",
          "This server supports OAuth. Sign-in completes in the system browser without exposing tokens to the frontend.",
        );
      case "unsupported":
        return props.copy(
          "该服务没有发布可用的 OAuth 元数据，可继续使用无认证或手动 Header。",
          "This server does not publish usable OAuth metadata; use no authentication or a manual header.",
        );
      default:
        return props.copy("正在检查认证能力…", "Checking authentication capabilities…");
    }
  };

  return (
    <section class="mcp-auth-panel" data-testid="mcp-auth-panel">
      <div class="mcp-auth-copy">
        <strong>{props.copy("认证", "Authentication")}</strong>
        <span>{description()}</span>
        <Show when={(props.status?.scopesSupported.length ?? 0) > 0}>
          <small>
            {props.copy("请求范围", "Requested scopes")}：{props.status?.scopesSupported.join(", ")}
          </small>
        </Show>
      </div>
      <Show when={props.status?.status === "not_logged_in"}>
        <Button
          size="small"
          variant="primary"
          data-testid="mcp-oauth-login"
          disabled={props.loading || !props.connectorEnabled}
          onClick={() => props.onLogin(props.status?.scopesSupported ?? [])}
        >
          {props.loading
            ? props.copy("等待登录…", "Waiting for sign-in…")
            : props.copy("使用 OAuth 登录", "Sign in with OAuth")}
        </Button>
      </Show>
      <Show when={props.status?.status === "oauth"}>
        <Button
          size="small"
          data-testid="mcp-oauth-logout"
          disabled={props.loading}
          onClick={props.onLogout}
        >
          {props.copy("退出登录", "Sign out")}
        </Button>
      </Show>
      <Show when={!props.status && !props.loading}>
        <StatusBanner tone="warning">
          {props.copy("无法确认此服务的认证状态。", "Authentication status is unavailable.")}
        </StatusBanner>
      </Show>
    </section>
  );
}
