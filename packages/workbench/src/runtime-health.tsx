import {
  commandFailure,
  commands,
  type RuntimeComponentHealth,
  type RuntimeComponentId,
  type RuntimeHealthSnapshot,
} from "@hachimi/contracts";
import { Button, RefreshCw, StatusBanner } from "@hachimi/ui";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";

const RUNTIME_HEALTH_EVENT = "runtime-health-changed";

const ERROR_COPY: Record<string, [string, string]> = {
  gateway_disabled: ["本地消息服务未启用。", "The local messaging service is disabled."],
  gateway_stopped: [
    "本地消息服务已停止，正在自动恢复。",
    "The local messaging service stopped and is recovering.",
  ],
  gateway_process_exited: [
    "本地消息服务异常退出，正在自动恢复。",
    "The local messaging service exited unexpectedly and is recovering.",
  ],
  gateway_process_start_failed: [
    "本地消息服务无法启动，请查看日志。",
    "The local messaging service could not start. Check the logs.",
  ],
  gateway_ready_timeout: [
    "本地消息服务启动超时，正在重试。",
    "The local messaging service timed out while starting and is retrying.",
  ],
  gateway_port_in_use: [
    "本地消息端口被其他程序占用。",
    "The local messaging port is in use by another process.",
  ],
  gateway_executable_lookup_failed: [
    "无法定位 Hachimi 本地消息程序。",
    "Hachimi could not locate its local messaging executable.",
  ],
  internal_resource_storage_failed: [
    "内部运行资源无法写入应用数据目录。",
    "Internal runtime resources could not be written to the app data directory.",
  ],
  internal_resource_invalid: [
    "内部运行资源校验失败。",
    "An internal runtime resource failed validation.",
  ],
  runtime_manifest_write_failed: [
    "内部运行资源清单无法保存。",
    "The internal runtime manifest could not be saved.",
  ],
  sandbox_setup_invalid: [
    "沙箱安装程序缺失或校验失败。",
    "The sandbox installer is missing or invalid.",
  ],
  sandbox_launcher_invalid: [
    "沙箱启动程序缺失或校验失败。",
    "The sandbox launcher is missing or invalid.",
  ],
  sandbox_attestation_invalid: [
    "沙箱安全校验组件缺失或损坏。",
    "The sandbox attestation component is missing or damaged.",
  ],
  workspace_worker_invalid: [
    "Workspace Worker 缺失或校验失败。",
    "The Workspace Worker is missing or invalid.",
  ],
  workspace_worker_registration_failed: [
    "Workspace Worker 无法注册。",
    "The Workspace Worker could not be registered.",
  ],
  managed_git_missing: ["内置 Git 不存在。", "Managed Git is missing."],
  managed_git_invalid: ["内置 Git 校验失败。", "Managed Git failed validation."],
  managed_git_registration_failed: ["内置 Git 无法注册。", "Managed Git could not be registered."],
  motion_catalog_unavailable: [
    "内置动作资源不可用，角色动作已降级。",
    "Bundled motion resources are unavailable; avatar motion is degraded.",
  ],
  voice_model_missing: [
    "内置语音模型不可用，语音能力已禁用。",
    "The bundled voice model is unavailable; voice is disabled.",
  ],
  speech_model_missing: [
    "语音识别模型不可用，语音输入已禁用。",
    "The speech recognition model is unavailable; voice input is disabled.",
  ],
  mcp_runtime_disabled: ["MCP Runtime 未启用。", "The MCP runtime is disabled."],
  mcp_server_unavailable: [
    "一个或多个 MCP 服务暂时不可用。",
    "One or more MCP servers are temporarily unavailable.",
  ],
  mcp_reconcile_failed: [
    "MCP 启动恢复失败，正在重试。",
    "MCP startup recovery failed and is retrying.",
  ],
  scheduler_disabled: ["计划任务 Runtime 未启用。", "The scheduled task runtime is disabled."],
  scheduler_storage_unavailable: [
    "计划任务存储暂时不可用。",
    "Scheduled task storage is temporarily unavailable.",
  ],
  scheduler_reconciliation_failed: ["计划任务恢复失败。", "Scheduled task recovery failed."],
  scheduler_restart_marker_failed: [
    "无法保存计划任务重启状态，已停止自动重启。",
    "The scheduler restart state could not be saved; automatic restart was stopped.",
  ],
  scheduler_restart_required: [
    "计划任务服务持续失败，Hachimi 将受控重启。",
    "The scheduled task service repeatedly failed; Hachimi will restart safely.",
  ],
  scheduler_restart_rate_limited: [
    "计划任务服务再次失败，已停止自动重启以避免循环。",
    "The scheduled task service failed again; automatic restart was stopped to prevent a loop.",
  ],
  browser_extension_not_connected: [
    "浏览器扩展尚未连接，内置浏览器仍可使用。",
    "The browser extension is not connected; the embedded browser remains available.",
  ],
  browser_extension_authorization_required: [
    "浏览器扩展正在等待一次授权确认。",
    "The browser extension is waiting for one-time authorization.",
  ],
  browser_extension_broker_unavailable: [
    "浏览器扩展本地连接服务不可用，已回退内置浏览器。",
    "The browser extension broker is unavailable; the embedded browser is being used.",
  ],
  cef_runtime_missing: [
    "内置浏览器 Runtime 缺失或损坏。",
    "The embedded browser runtime is missing or damaged.",
  ],
  cef_start_failed: ["内置浏览器无法启动。", "The embedded browser could not start."],
  cef_ready_timeout: ["内置浏览器启动超时。", "The embedded browser timed out while starting."],
  cef_runtime_crashed: [
    "内置浏览器异常退出，正在恢复。",
    "The embedded browser crashed and is recovering.",
  ],
  cef_ipc_failed: ["内置浏览器通信失败。", "Communication with the embedded browser failed."],
  cef_command_timeout: ["内置浏览器响应超时。", "The embedded browser did not respond in time."],
  cef_window_unavailable: ["内置浏览器窗口不可用。", "The embedded browser window is unavailable."],
  cef_restart_exhausted: [
    "内置浏览器连续恢复失败，自动模式将尝试已授权的系统浏览器。",
    "Embedded browser recovery was exhausted; auto mode will try an authorized system browser.",
  ],
  computer_capture_unavailable: [
    "当前系统不支持窗口画面捕获。",
    "Window capture is not available on this system.",
  ],
  computer_protected_desktop: [
    "当前处于锁屏或受保护桌面，Computer Use 已暂停。",
    "Computer Use is paused on the lock screen or a protected desktop.",
  ],
};

const COMPONENT_COPY: Record<RuntimeComponentId, [string, string]> = {
  gateway: ["本地消息服务", "Local messaging service"],
  internal_resources: ["内部运行资源", "Internal runtime resources"],
  mcp: ["MCP 服务", "MCP services"],
  scheduler: ["计划任务服务", "Scheduled task service"],
  browser_extension: ["浏览器扩展", "Browser extension"],
  cef: ["内置浏览器", "Embedded browser"],
  computer_use: ["Computer Use", "Computer Use"],
};

export function runtimeErrorMessage(code: string | null, zh: boolean): string {
  if (!code)
    return zh ? "运行状态异常，请重试。" : "The runtime is unhealthy. Retry the operation.";
  return (ERROR_COPY[code] ?? [
    "该能力暂时不可用，请重试或查看日志。",
    "This capability is temporarily unavailable. Retry or check the logs.",
  ])[zh ? 0 : 1];
}

export function runtimeComponentLabel(component: RuntimeComponentId, zh: boolean): string {
  return COMPONENT_COPY[component][zh ? 0 : 1];
}

export function RuntimeHealthBanner(props: {
  component: RuntimeComponentId;
  zh: boolean;
  showReady?: boolean;
}) {
  const [health, setHealth] = createSignal<RuntimeComponentHealth>();
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  let unlisten: UnlistenFn | undefined;
  let disposed = false;

  const visible = createMemo(() => props.showReady || health()?.state !== "ready");
  const tone = createMemo(() => {
    const state = health()?.state;
    if (state === "ready") return "success" as const;
    if (state === "failed") return "danger" as const;
    return "warning" as const;
  });

  function select(snapshot: RuntimeHealthSnapshot) {
    const next = snapshot.components.find((entry) => entry.component === props.component);
    if (next) setHealth(next);
  }

  async function retry() {
    setBusy(true);
    setFailure();
    try {
      select(await commands.retryRuntimeComponent(props.component));
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  onMount(() => {
    if (
      typeof window === "undefined" ||
      !("__TAURI_INTERNALS__" in window) ||
      typeof commands.getRuntimeHealth !== "function"
    )
      return;
    void commands
      .getRuntimeHealth()
      .then(select)
      .catch((error) => {
        setFailure(commandFailure(error).message);
      });
    // eslint-disable-next-line solid/reactivity -- Tauri invokes this external event callback.
    void listen<RuntimeHealthSnapshot>(RUNTIME_HEALTH_EVENT, ({ payload }) => select(payload)).then(
      (stop) => {
        if (disposed) stop();
        else unlisten = stop;
      },
    );
  });

  onCleanup(() => {
    disposed = true;
    unlisten?.();
  });

  return (
    <Show when={visible() && (health() || failure())}>
      <StatusBanner tone={failure() ? "danger" : tone()}>
        <div class="runtime-health-banner" data-runtime-component={props.component}>
          <span>
            <strong>{runtimeComponentLabel(props.component, props.zh)}</strong>
            <small>
              {failure() ??
                (health()?.state === "ready"
                  ? props.zh
                    ? "已就绪"
                    : "Ready"
                  : runtimeErrorMessage(health()?.errorCode ?? null, props.zh))}
            </small>
          </span>
          <Show when={health()?.retryable}>
            <Button size="small" disabled={busy()} onClick={() => void retry()}>
              <RefreshCw size={13} />
              {busy() ? (props.zh ? "正在重试" : "Retrying") : props.zh ? "立即重试" : "Retry now"}
            </Button>
          </Show>
        </div>
      </StatusBanner>
    </Show>
  );
}

export function mcpHealthMessage(code: string | null, zh: boolean): string {
  const copy: Record<string, [string, string]> = {
    invalid_configuration: ["配置无效", "Invalid configuration"],
    spawn_failed: ["程序不存在或无法启动", "Program not found or could not start"],
    disconnected: ["连接已断开", "Connection closed"],
    timeout: ["连接超时", "Connection timed out"],
    transport_error: ["连接失败", "Connection failed"],
    invalid_response: ["服务返回了无效响应", "The server returned an invalid response"],
    unsupported_protocol: ["协议版本不受支持", "Unsupported protocol version"],
    mcp_credential_unavailable: ["凭据不可用", "Credentials unavailable"],
    mcp_authentication_conflict: ["认证配置冲突", "Authentication configuration conflict"],
    capability_registration_failed: ["Tools 注册失败", "Tool registration failed"],
    runtime_not_loaded: ["服务尚未加载", "The service is not loaded"],
  };
  return (copy[code ?? ""] ?? ["服务暂时不可用", "Service temporarily unavailable"])[zh ? 0 : 1];
}
