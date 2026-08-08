import {
  commandFailure,
  commands,
  type McpConnectionTestResult,
  type McpAuthStatusRecord,
  type McpCallSummaryRecord,
  type McpHeaderInput,
  type McpInventorySnapshot,
  type McpServerDraft,
  type McpServerView,
  type McpToolView,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  Button,
  Dialog,
  PageHeading,
  Plus,
  RefreshCw,
  SegmentedControl,
  StatusBanner,
  Switch as Toggle,
  TextArea,
  TextField,
  Trash2,
  Workspace,
} from "@hachimi/ui";
import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";

import { McpInventoryPanel } from "./mcp-inventory-panel";
import { McpCallHistory } from "./mcp-call-history";
import { McpAuthPanel } from "./mcp-auth-panel";
import { RuntimeHealthBanner, mcpHealthMessage } from "./runtime-health";

export function McpSettingsPage(props: { connectorEnabled: boolean }) {
  const i18n = useI18n();
  const copy = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const [servers, setServers] = createSignal<McpServerView[]>([]);
  const [selectedId, setSelectedId] = createSignal<string>();
  const [draft, setDraft] = createSignal<McpServerDraft>(emptyDraft());
  const [tools, setTools] = createSignal<McpToolView[]>([]);
  const [testResult, setTestResult] = createSignal<McpConnectionTestResult>();
  const [loading, setLoading] = createSignal(true);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string>();
  const [dirty, setDirty] = createSignal(false);
  const [createOpen, setCreateOpen] = createSignal(false);
  const [createDraft, setCreateDraft] = createSignal<McpServerDraft>(emptyDraft());
  const [createTestResult, setCreateTestResult] = createSignal<McpConnectionTestResult>();
  const [createBusy, setCreateBusy] = createSignal(false);
  const [echoUrl, setEchoUrl] = createSignal<string>();
  const [toolsLoading, setToolsLoading] = createSignal(false);
  const [inventory, setInventory] = createSignal<McpInventorySnapshot>();
  const [inventoryLoading, setInventoryLoading] = createSignal(false);
  const [callSummaries, setCallSummaries] = createSignal<McpCallSummaryRecord[]>([]);
  const [authStatus, setAuthStatus] = createSignal<McpAuthStatusRecord>();
  const [authBusy, setAuthBusy] = createSignal(false);
  const [deleteTarget, setDeleteTarget] = createSignal<McpServerView>();
  let selectionGeneration = 0;

  const selected = createMemo(() =>
    servers().find((server) => server.configuration.id === selectedId()),
  );
  const httpTransport = createMemo(() => {
    const transport = draft().transport;
    return transport.kind === "streamable_http" ? transport : undefined;
  });
  const stdioTransport = createMemo(() => {
    const transport = draft().transport;
    return transport.kind === "stdio" ? transport : undefined;
  });

  async function reload(selectId?: string) {
    const next = await commands.listMcpServers();
    setServers(next);
    const id = selectId ?? selectedId() ?? next[0]?.configuration.id;
    if (id) await selectServer(id, true);
  }

  async function selectServer(serverId: string, force = false) {
    if (
      !force &&
      dirty() &&
      !window.confirm(
        copy(
          "当前 MCP 配置尚未保存，确定切换吗？",
          "Discard unsaved MCP changes and switch services?",
        ),
      )
    )
      return;
    const generation = ++selectionGeneration;
    const [view, cached, cachedInventory, recentCalls] = await Promise.all([
      commands.getMcpServer(serverId),
      commands.listMcpTools(serverId),
      commands.getMcpInventory(serverId),
      commands.listMcpCallSummaries({ serverId, sessionId: null, limit: 25 }),
    ]);
    if (generation !== selectionGeneration) return;
    setSelectedId(serverId);
    setDraft(draftFromView(view));
    setDirty(false);
    setTestResult();
    setToolsLoading(true);
    setTools(cached);
    setInventory(cachedInventory);
    setCallSummaries(recentCalls);
    setAuthStatus();
    void loadAuthStatus(serverId, generation);
    if (!props.connectorEnabled) {
      setToolsLoading(false);
      return;
    }
    try {
      const result = await commands.discoverMcpTools(serverId);
      if (generation !== selectionGeneration) return;
      setTestResult(result.success ? undefined : result);
      if (result.success) setTools(result.tools);
    } catch (reason) {
      if (generation === selectionGeneration) setError(commandFailure(reason).message);
    } finally {
      if (generation === selectionGeneration) setToolsLoading(false);
    }
    if (generation === selectionGeneration) {
      setInventoryLoading(true);
      try {
        const refreshed = await commands.refreshMcpInventory(serverId);
        if (generation === selectionGeneration) setInventory(refreshed);
      } catch (reason) {
        if (generation === selectionGeneration) setError(commandFailure(reason).message);
      } finally {
        if (generation === selectionGeneration) setInventoryLoading(false);
      }
    }
  }

  async function loadAuthStatus(serverId: string, generation = selectionGeneration) {
    try {
      const status = await commands.getMcpAuthStatus(serverId);
      if (generation === selectionGeneration) setAuthStatus(status);
    } catch (reason) {
      if (generation === selectionGeneration) {
        setAuthStatus();
        setError(commandFailure(reason).message);
      }
    }
  }

  async function loginOAuth(scopes: string[]) {
    const serverId = selectedId();
    if (!serverId) return;
    const generation = selectionGeneration;
    setAuthBusy(true);
    setError();
    try {
      const response = await commands.startMcpOAuthLogin({
        serverId,
        scopes,
        timeoutSecs: 300,
      });
      window.open(response.authorizationUrl, "_blank", "noopener,noreferrer");
      for (let attempt = 0; attempt < 300 && generation === selectionGeneration; attempt += 1) {
        await new Promise((resolve) => window.setTimeout(resolve, 1_000));
        const status = await commands.getMcpAuthStatus(serverId);
        if (generation !== selectionGeneration) return;
        setAuthStatus(status);
        if (status.status === "oauth") {
          await commands.refreshMcpServer(serverId).catch(() => undefined);
          await commands
            .refreshMcpInventory(serverId)
            .then(setInventory)
            .catch(() => undefined);
          return;
        }
      }
    } catch (reason) {
      if (generation === selectionGeneration) setError(commandFailure(reason).message);
    } finally {
      if (generation === selectionGeneration) setAuthBusy(false);
    }
  }

  async function logoutOAuth() {
    const serverId = selectedId();
    if (!serverId) return;
    const generation = selectionGeneration;
    setAuthBusy(true);
    setError();
    try {
      const status = await commands.logoutMcpOAuth(serverId);
      if (generation === selectionGeneration) setAuthStatus(status);
    } catch (reason) {
      if (generation === selectionGeneration) setError(commandFailure(reason).message);
    } finally {
      if (generation === selectionGeneration) setAuthBusy(false);
    }
  }

  async function refreshInventory() {
    const serverId = selectedId();
    if (!serverId) return;
    const generation = selectionGeneration;
    setInventoryLoading(true);
    setError();
    try {
      const refreshed = await commands.refreshMcpInventory(serverId);
      if (generation === selectionGeneration) setInventory(refreshed);
    } catch (reason) {
      if (generation === selectionGeneration) setError(commandFailure(reason).message);
    } finally {
      if (generation === selectionGeneration) setInventoryLoading(false);
    }
  }

  function update(patch: Partial<McpServerDraft>) {
    setDraft((current) => ({ ...current, ...patch }));
    setDirty(true);
    setTestResult();
  }

  function newServer() {
    setCreateDraft(emptyDraft());
    setCreateTestResult();
    setCreateOpen(true);
  }

  function updateCreate(patch: Partial<McpServerDraft>) {
    setCreateDraft((current) => ({ ...current, ...patch }));
    setCreateTestResult();
  }

  async function testCreate() {
    setCreateBusy(true);
    setError();
    setCreateTestResult();
    try {
      setCreateTestResult(await commands.testMcpServer({ ...createDraft(), enabled: true }));
    } catch (reason) {
      setError(commandFailure(reason).message);
    } finally {
      setCreateBusy(false);
    }
  }

  async function saveCreate() {
    setCreateBusy(true);
    setError();
    try {
      const saved = await commands.upsertMcpServer({ ...createDraft(), enabled: false });
      setCreateOpen(false);
      await reload(saved.configuration.id);
    } catch (reason) {
      setError(commandFailure(reason).message);
    } finally {
      setCreateBusy(false);
    }
  }

  async function save() {
    setBusy(true);
    setError();
    try {
      const saved = await commands.upsertMcpServer({
        ...draft(),
        enabled: selected()?.configuration.enabled ?? false,
      });
      setDirty(false);
      await reload(saved.configuration.id);
    } catch (reason) {
      setError(commandFailure(reason).message);
    } finally {
      setBusy(false);
    }
  }

  async function test() {
    setBusy(true);
    setError();
    setTestResult();
    try {
      const result = await commands.testMcpServer({ ...draft(), enabled: true });
      setTestResult(result);
    } catch (reason) {
      setError(commandFailure(reason).message);
    } finally {
      setBusy(false);
    }
  }

  async function toggleServer(enabled: boolean) {
    const serverId = selectedId();
    if (!serverId) return;
    try {
      await commands.setMcpServerEnabled(serverId, enabled);
      await reload(serverId);
    } catch (reason) {
      setError(commandFailure(reason).message);
    }
  }

  async function retrySelected() {
    const serverId = selectedId();
    if (!serverId) return;
    setBusy(true);
    setError();
    try {
      await commands.refreshMcpServer(serverId);
      await reload(serverId);
    } catch (reason) {
      setError(commandFailure(reason).message);
    } finally {
      setBusy(false);
    }
  }

  async function remove(view: McpServerView) {
    try {
      await commands.removeMcpServer(view.configuration.id);
      if (selectedId() === view.configuration.id) {
        selectionGeneration += 1;
        setSelectedId();
        setDraft(emptyDraft());
        setTools([]);
        setInventory();
        setCallSummaries([]);
        setAuthStatus();
      }
      setDeleteTarget();
      await reload();
    } catch (reason) {
      setError(commandFailure(reason).message);
    }
  }

  async function setToolEnabled(tool: McpToolView, enabled: boolean) {
    try {
      const updated = await commands.setMcpToolEnabled(tool.serverId, tool.name, enabled);
      setTools((current) => current.map((item) => (item.name === updated.name ? updated : item)));
    } catch (reason) {
      setError(commandFailure(reason).message);
    }
  }

  function addHeader() {
    update({ headers: [...draft().headers, { name: "", value: "", secret: false }] });
  }

  function updateHeader(index: number, patch: Partial<McpHeaderInput>) {
    update({
      headers: draft().headers.map((header, current) =>
        current === index ? { ...header, ...patch } : header,
      ),
    });
  }

  function removeHeader(index: number) {
    update({ headers: draft().headers.filter((_, current) => current !== index) });
  }

  onMount(() => {
    void reload()
      .catch((reason) => setError(commandFailure(reason).message))
      .finally(() => setLoading(false));
    void commands
      .getMcpEchoServerUrl()
      .then(setEchoUrl)
      .catch(() => undefined);
  });

  onCleanup(() => {
    selectionGeneration += 1;
  });

  return (
    <div class="extension-settings-page" data-testid="mcp-settings-page">
      <PageHeading
        class="extension-page-heading"
        title="MCP"
        description={copy(
          "连接 Streamable HTTP 或本地 stdio 服务，并精确控制向 Agent 暴露的 Tools。",
          "Connect Streamable HTTP or local stdio servers and control exactly which Tools are exposed to the Agent.",
        )}
        actions={
          <Button variant="primary" data-testid="mcp-add-server" onClick={newServer}>
            <Plus size={15} /> {copy("新增 MCP 服务", "Add MCP server")}
          </Button>
        }
      />
      <Show when={!props.connectorEnabled}>
        <StatusBanner tone="warning">
          {copy(
            "Connector Runtime 当前未启用。可以编辑配置，但测试、启用和调用 Tools 需要开启功能开关。",
            "The Connector Runtime is disabled. You can edit configurations, but testing, enabling, and calling Tools requires the feature flag.",
          )}
        </StatusBanner>
      </Show>
      <Show when={error()}>
        {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
      </Show>
      <RuntimeHealthBanner component="mcp" zh={i18n.locale() === "zh-CN"} />
      <Workspace class="extension-workspace">
        <aside class="extension-sidebar">
          <div class="extension-panel-toolbar">
            <strong>{copy("MCP 服务", "MCP servers")}</strong>
            <span>{servers().length}</span>
          </div>
          <div class="extension-list">
            <Show
              when={!loading()}
              fallback={<div class="extension-empty">{copy("正在加载 MCP…", "Loading MCP…")}</div>}
            >
              <For each={servers()}>
                {(server) => (
                  <div
                    class="mcp-server-row"
                    classList={{ selected: selectedId() === server.configuration.id }}
                  >
                    <Button
                      type="button"
                      class="mcp-server-select"
                      data-testid={`mcp-server-${server.configuration.id}`}
                      onClick={() => void selectServer(server.configuration.id)}
                    >
                      <span class="extension-status-dot" data-state={server.health.state} />
                      <span>
                        <strong>{server.configuration.displayName}</strong>
                        <small>{serverSubtitle(server)}</small>
                      </span>
                    </Button>
                    <Button
                      type="button"
                      class="mcp-server-delete"
                      aria-label={copy(
                        `删除 ${server.configuration.displayName}`,
                        `Delete ${server.configuration.displayName}`,
                      )}
                      onClick={() => setDeleteTarget(server)}
                    >
                      <Trash2 size={13} />
                    </Button>
                  </div>
                )}
              </For>
            </Show>
          </div>
        </aside>
        <main class="extension-main">
          <section class="mcp-detail">
            <header class="mcp-detail-header">
              <div>
                <strong>
                  {selected()?.configuration.displayName ?? copy("新增 MCP 服务", "New MCP server")}
                </strong>
                <small>
                  {selected()?.health.errorCode
                    ? mcpHealthMessage(selected()!.health.errorCode, i18n.locale() === "zh-CN")
                    : (selected()?.health.state ?? copy("尚未保存", "Not saved"))}
                </small>
              </div>
              <div class="extension-toolbar-actions">
                <Show when={selected()}>
                  <Show
                    when={
                      selected()!.configuration.enabled && selected()!.health.state === "failed"
                    }
                  >
                    <Button disabled={busy()} onClick={() => void retrySelected()}>
                      <RefreshCw size={14} /> {copy("立即重试", "Retry now")}
                    </Button>
                  </Show>
                  <div class="mcp-server-exposure">
                    <div>
                      <strong>{copy("提供给 Agent", "Available to Agent")}</strong>
                      <span>
                        {copy(
                          "启用后，允许的 Tools 才会注册到 Agent。",
                          "When enabled, allowed Tools are registered for the Agent.",
                        )}
                      </span>
                    </div>
                    <Toggle
                      label={copy("启用 MCP 服务", "Enable MCP server")}
                      checked={selected()?.configuration.enabled ?? false}
                      disabled={!props.connectorEnabled}
                      onChange={(enabled) => void toggleServer(enabled)}
                    />
                  </div>
                </Show>
              </div>
            </header>
            <Show
              when={selected()}
              fallback={
                <div class="extension-empty mcp-select-empty">
                  {copy(
                    "从左侧选择一个 MCP 服务，或点击右上角新增服务。",
                    "Select an MCP server on the left, or add one from the top right.",
                  )}
                </div>
              }
            >
              <div class="mcp-form">
                <details class="mcp-advanced-transport" open={draft().transport.kind === "stdio"}>
                  <summary>{copy("高级本地服务", "Advanced local service")}</summary>
                  <p>
                    {copy(
                      "默认使用 Streamable HTTP。仅在需要启动本地进程时切换到 stdio。",
                      "Streamable HTTP is the default. Switch to stdio only for a local process.",
                    )}
                  </p>
                  <SegmentedControl
                    label="MCP Transport"
                    value={draft().transport.kind}
                    options={[
                      { value: "streamable_http", label: copy("远程 URL", "Remote URL") },
                      { value: "stdio", label: copy("本地 stdio", "Local stdio") },
                    ]}
                    onChange={(kind) =>
                      update({
                        transport:
                          kind === "streamable_http"
                            ? { kind, url: "" }
                            : { kind, command: "", args: [], cwd: null },
                      })
                    }
                  />
                </details>
                <div class="mcp-form-grid">
                  <TextField
                    label={copy("名称", "Name")}
                    value={draft().displayName}
                    placeholder={copy("例如 Filesystem", "For example, Filesystem")}
                    onInput={(event) => update({ displayName: event.currentTarget.value })}
                  />
                  <Show
                    when={draft().transport.kind === "streamable_http"}
                    fallback={
                      <TextField
                        label="Command"
                        value={stdioTransport()?.command ?? ""}
                        placeholder="npx"
                        onInput={(event) => {
                          const transport = draft().transport;
                          if (transport.kind === "stdio")
                            update({
                              transport: { ...transport, command: event.currentTarget.value },
                            });
                        }}
                      />
                    }
                  >
                    <TextField
                      label="URL"
                      value={httpTransport()?.url ?? ""}
                      placeholder="https://example.com/mcp"
                      onInput={(event) => {
                        const transport = draft().transport;
                        if (transport.kind === "streamable_http")
                          update({ transport: { ...transport, url: event.currentTarget.value } });
                      }}
                    />
                  </Show>
                </div>
                <Show when={draft().transport.kind === "stdio"}>
                  <div class="mcp-form-grid">
                    <TextArea
                      label={copy("Args（每行一个）", "Args (one per line)")}
                      value={stdioTransport()?.args.join("\n") ?? ""}
                      onInput={(event) => {
                        const transport = draft().transport;
                        if (transport.kind === "stdio")
                          update({
                            transport: {
                              ...transport,
                              args: event.currentTarget.value.split("\n").filter(Boolean),
                            },
                          });
                      }}
                    />
                    <TextField
                      label={copy("Working directory（可选）", "Working directory (optional)")}
                      value={stdioTransport()?.cwd ?? ""}
                      onInput={(event) => {
                        const transport = draft().transport;
                        if (transport.kind === "stdio")
                          update({
                            transport: { ...transport, cwd: event.currentTarget.value || null },
                          });
                      }}
                    />
                  </div>
                </Show>
                <Show when={draft().transport.kind === "streamable_http"}>
                  <div class="mcp-header-list">
                    <div class="mcp-tool-header">
                      <div class="mcp-header-heading">
                        <strong>Headers</strong>
                        <span>
                          {copy(
                            "凭据类 Header 会自动保存到系统安全凭据库。",
                            "Credential headers are stored automatically in the OS keyring.",
                          )}
                        </span>
                      </div>
                      <Button size="small" onClick={addHeader}>
                        <Plus size={13} /> {copy("添加 Header", "Add Header")}
                      </Button>
                    </div>
                    <For each={draft().headers}>
                      {(header, index) => (
                        <div class="mcp-header-row">
                          <TextField
                            label="Header"
                            value={header.name}
                            placeholder="Authorization"
                            onInput={(event) =>
                              updateHeader(index(), { name: event.currentTarget.value })
                            }
                          />
                          <TextField
                            label={copy("值", "Value")}
                            type={header.secret ? "password" : "text"}
                            value={header.value ?? ""}
                            placeholder={
                              header.value === null
                                ? copy("保持已保存的值", "Keep saved value")
                                : "Bearer …"
                            }
                            onInput={(event) =>
                              updateHeader(index(), { value: event.currentTarget.value })
                            }
                          />
                          <Button
                            size="small"
                            variant="danger"
                            onClick={() => removeHeader(index())}
                          >
                            <Trash2 size={13} />
                          </Button>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
                <Show when={testResult()}>
                  {(result) => (
                    <StatusBanner tone={result().success ? "success" : "danger"}>
                      {result().success
                        ? copy(
                            `连接成功，发现 ${result().tools.length} 个 Tools。`,
                            `Connected. Discovered ${result().tools.length} Tools.`,
                          )
                        : copy(
                            `连接失败：${result().errorCode ?? "unknown"}`,
                            `Connection failed: ${result().errorCode ?? "unknown"}`,
                          )}
                    </StatusBanner>
                  )}
                </Show>
                <div class="mcp-form-actions">
                  <Button
                    data-testid="mcp-test-connection"
                    disabled={busy() || !props.connectorEnabled}
                    onClick={() => void test()}
                  >
                    {copy("测试连接", "Test connection")}
                  </Button>
                  <Button
                    data-testid="mcp-save-server"
                    variant="primary"
                    disabled={busy() || !draftValid(draft())}
                    onClick={() => void save()}
                  >
                    {busy() ? copy("处理中…", "Working…") : copy("保存", "Save")}
                  </Button>
                </div>
              </div>
            </Show>
            <Show when={selected()}>
              <McpAuthPanel
                status={authStatus()}
                loading={authBusy()}
                connectorEnabled={props.connectorEnabled}
                copy={copy}
                onLogin={(scopes) => void loginOAuth(scopes)}
                onLogout={() => void logoutOAuth()}
              />
            </Show>
            <Show when={selected()}>
              <div class="mcp-tools">
                <div class="mcp-tools-heading">
                  <div>
                    <h2>Tools · {tools().length}</h2>
                    <span>
                      {copy(
                        "选择服务后自动读取。关闭某个 Tool 后，下一次模型请求将不再看到它。",
                        "Loaded when selected. Disabled Tools disappear from the next model request.",
                      )}
                    </span>
                  </div>
                  <Show when={toolsLoading()}>
                    <span class="mcp-tools-loading">{copy("正在读取…", "Loading…")}</span>
                  </Show>
                </div>
                <Show
                  when={tools().length > 0}
                  fallback={
                    <div class="extension-empty">
                      {toolsLoading()
                        ? copy("正在连接服务并读取 Tools…", "Connecting and loading Tools…")
                        : copy("该服务没有可用 Tools。", "This server has no available Tools.")}
                    </div>
                  }
                >
                  <For each={tools()}>
                    {(tool) => (
                      <article
                        class="mcp-tool-card"
                        data-component="mcp-card"
                        data-testid={`mcp-tool-${tool.name}`}
                      >
                        <div class="mcp-tool-card-header">
                          <div class="mcp-tool-copy">
                            <div class="mcp-tool-title">
                              <code>{tool.name}</code>
                              <Show when={tool.stale}>
                                <span data-tone="warning">{copy("缓存", "Cached")}</span>
                              </Show>
                              <Show when={tool.validationError}>
                                <span data-tone="danger">{copy("不可用", "Invalid")}</span>
                              </Show>
                            </div>
                            <p>
                              {tool.description ??
                                copy("服务未提供描述", "No description supplied")}
                            </p>
                            <span class="mcp-tool-model-name">
                              {copy("暴露名称", "Exposed name")}：<code>{tool.exposedName}</code>
                            </span>
                            <Show when={tool.validationError}>
                              {(message) => (
                                <span class="mcp-tool-error">
                                  {copy("无效 Schema", "Invalid Schema")}：{message()}
                                </span>
                              )}
                            </Show>
                          </div>
                          <div class="mcp-tool-exposure">
                            <div>
                              <strong>{copy("提供给 Agent", "Expose to Agent")}</strong>
                              <span>
                                {tool.enabled
                                  ? copy("Agent 可以调用此工具", "The Agent can call this Tool")
                                  : copy("此工具不会发送给模型", "This Tool is hidden from models")}
                              </span>
                            </div>
                            <Toggle
                              label={copy(
                                `向 Agent 暴露 ${tool.name}`,
                                `Expose ${tool.name} to the Agent`,
                              )}
                              checked={tool.enabled}
                              disabled={tool.validationError !== null}
                              onChange={(enabled) => void setToolEnabled(tool, enabled)}
                            />
                          </div>
                        </div>
                        <ToolParameters tool={tool} />
                        <details>
                          <summary>{copy("参数 Schema", "Parameter Schema")}</summary>
                          <pre class="mcp-tool-schema">
                            {JSON.stringify(tool.inputSchema, null, 2)}
                          </pre>
                        </details>
                      </article>
                    )}
                  </For>
                </Show>
              </div>
            </Show>
            <Show when={selected()}>
              {(view) => (
                <McpInventoryPanel
                  snapshot={inventory()}
                  loading={inventoryLoading()}
                  connectorEnabled={props.connectorEnabled}
                  runtimeReady={view().health.state === "ready"}
                  onRefresh={refreshInventory}
                />
              )}
            </Show>
            <Show when={selected()}>
              <McpCallHistory calls={callSummaries()} copy={copy} />
            </Show>
          </section>
        </main>
      </Workspace>
      <Dialog
        open={Boolean(deleteTarget())}
        title={copy("删除 MCP 服务", "Delete MCP server")}
        description={copy(
          `确定删除 MCP 服务“${deleteTarget()?.configuration.displayName ?? ""}”吗？`,
          `Delete MCP server “${deleteTarget()?.configuration.displayName ?? ""}”?`,
        )}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget();
        }}
        closeLabel={copy("关闭", "Close")}
      >
        <div class="dialog-confirmation-actions">
          <Button onClick={() => setDeleteTarget()}>{copy("取消", "Cancel")}</Button>
          <Button
            variant="danger"
            onClick={() => {
              const target = deleteTarget();
              if (target) void remove(target);
            }}
          >
            {copy("删除", "Delete")}
          </Button>
        </div>
      </Dialog>
      <Dialog
        open={createOpen()}
        title={copy("新增 MCP 服务", "Add MCP server")}
        description={copy(
          "填写服务名称、Streamable HTTP URL 和可选 Headers。测试连接不会保存草稿。",
          "Enter a name, Streamable HTTP URL, and optional headers. Testing does not save the draft.",
        )}
        size="wide"
        onOpenChange={setCreateOpen}
        closeLabel={copy("关闭", "Close")}
      >
        <McpCreateForm
          draft={createDraft()}
          busy={createBusy()}
          connectorEnabled={props.connectorEnabled}
          echoUrl={echoUrl()}
          testResult={createTestResult()}
          error={error()}
          copy={copy}
          onUpdate={updateCreate}
          onTest={() => void testCreate()}
          onSave={() => void saveCreate()}
          onCancel={() => {
            setCreateOpen(false);
          }}
        />
      </Dialog>
    </div>
  );
}

function McpCreateForm(props: {
  draft: McpServerDraft;
  busy: boolean;
  connectorEnabled: boolean;
  echoUrl: string | undefined;
  testResult: McpConnectionTestResult | undefined;
  error: string | undefined;
  copy: (zh: string, en: string) => string;
  onUpdate: (patch: Partial<McpServerDraft>) => void;
  onTest: () => void;
  onSave: () => void;
  onCancel: () => void;
}) {
  const httpUrl = createMemo(() =>
    props.draft.transport.kind === "streamable_http" ? props.draft.transport.url : "",
  );
  const stdioTransport = createMemo(() =>
    props.draft.transport.kind === "stdio" ? props.draft.transport : undefined,
  );

  function updateHeader(index: number, patch: Partial<McpHeaderInput>) {
    props.onUpdate({
      headers: props.draft.headers.map((header, current) =>
        current === index ? { ...header, ...patch } : header,
      ),
    });
  }

  return (
    <div class="dialog-form mcp-create-dialog-form">
      <Show when={props.echoUrl}>
        {(url) => (
          <div class="mcp-echo-helper">
            <div>
              <strong>{props.copy("内置 Echo 测试服务", "Built-in Echo test server")}</strong>
              <code>{url()}</code>
            </div>
            <Button
              size="small"
              data-testid="mcp-use-echo-server"
              onClick={() =>
                props.onUpdate({
                  displayName: "Hachimi Echo",
                  transport: { kind: "streamable_http", url: url() },
                })
              }
            >
              {props.copy("使用此地址", "Use this address")}
            </Button>
          </div>
        )}
      </Show>
      <SegmentedControl
        label="MCP Transport"
        value={props.draft.transport.kind}
        options={[
          { value: "streamable_http", label: props.copy("远程 URL", "Remote URL") },
          { value: "stdio", label: props.copy("本地 stdio", "Local stdio") },
        ]}
        onChange={(kind) =>
          props.onUpdate({
            transport:
              kind === "streamable_http"
                ? { kind, url: "" }
                : { kind, command: "", args: [], cwd: null },
          })
        }
      />
      <div class="mcp-form-grid">
        <TextField
          testId="mcp-create-name"
          label={props.copy("名称", "Name")}
          value={props.draft.displayName}
          placeholder={props.copy("例如 Filesystem", "For example, Filesystem")}
          onInput={(event) => props.onUpdate({ displayName: event.currentTarget.value })}
        />
        <Show
          when={props.draft.transport.kind === "streamable_http"}
          fallback={
            <TextField
              testId="mcp-create-stdio-command"
              label="Command"
              value={stdioTransport()?.command ?? ""}
              placeholder="node.exe"
              onInput={(event) => {
                const transport = props.draft.transport;
                if (transport.kind === "stdio")
                  props.onUpdate({
                    transport: { ...transport, command: event.currentTarget.value },
                  });
              }}
            />
          }
        >
          <TextField
            label="URL"
            value={httpUrl()}
            placeholder="https://example.com/mcp"
            onInput={(event) =>
              props.onUpdate({
                transport: { kind: "streamable_http", url: event.currentTarget.value },
              })
            }
          />
        </Show>
      </div>
      <Show when={props.draft.transport.kind === "stdio"}>
        <div class="mcp-form-grid">
          <TextArea
            data-testid="mcp-create-stdio-args"
            label={props.copy("Args（每行一个）", "Args (one per line)")}
            value={stdioTransport()?.args.join("\n") ?? ""}
            onInput={(event) => {
              const transport = props.draft.transport;
              if (transport.kind === "stdio")
                props.onUpdate({
                  transport: {
                    ...transport,
                    args: event.currentTarget.value.split("\n").filter(Boolean),
                  },
                });
            }}
          />
          <TextField
            testId="mcp-create-stdio-cwd"
            label={props.copy("Working directory（可选）", "Working directory (optional)")}
            value={stdioTransport()?.cwd ?? ""}
            onInput={(event) => {
              const transport = props.draft.transport;
              if (transport.kind === "stdio")
                props.onUpdate({
                  transport: { ...transport, cwd: event.currentTarget.value || null },
                });
            }}
          />
        </div>
      </Show>
      <Show when={props.draft.transport.kind === "streamable_http"}>
        <div class="mcp-header-list">
          <div class="mcp-tool-header">
            <div class="mcp-header-heading">
              <strong>Headers</strong>
              <span>
                {props.copy(
                  "Authorization、Cookie、Token 等敏感 Header 会自动保存到系统凭据库。",
                  "Sensitive headers such as Authorization, Cookie, and tokens are stored in the OS keyring automatically.",
                )}
              </span>
            </div>
            <Button
              size="small"
              onClick={() =>
                props.onUpdate({
                  headers: [...props.draft.headers, { name: "", value: "", secret: false }],
                })
              }
            >
              <Plus size={13} /> {props.copy("添加 Header", "Add Header")}
            </Button>
          </div>
          <For each={props.draft.headers}>
            {(header, index) => (
              <div class="mcp-header-row">
                <TextField
                  label="Header"
                  value={header.name}
                  placeholder="Authorization"
                  onInput={(event) => updateHeader(index(), { name: event.currentTarget.value })}
                />
                <TextField
                  label={props.copy("值", "Value")}
                  type={header.secret ? "password" : "text"}
                  value={header.value ?? ""}
                  placeholder="Bearer …"
                  onInput={(event) => updateHeader(index(), { value: event.currentTarget.value })}
                />
                <Button
                  size="small"
                  variant="danger"
                  onClick={() =>
                    props.onUpdate({
                      headers: props.draft.headers.filter((_, current) => current !== index()),
                    })
                  }
                >
                  <Trash2 size={13} />
                </Button>
              </div>
            )}
          </For>
        </div>
      </Show>
      <Show when={props.error}>
        {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
      </Show>
      <Show when={props.testResult}>
        {(result) => (
          <>
            <StatusBanner tone={result().success ? "success" : "danger"}>
              {result().success
                ? props.copy(
                    `连接成功，发现 ${result().tools.length} 个 Tools。`,
                    `Connected. Discovered ${result().tools.length} Tools.`,
                  )
                : props.copy(
                    `连接失败：${result().errorCode ?? "unknown"}`,
                    `Connection failed: ${result().errorCode ?? "unknown"}`,
                  )}
            </StatusBanner>
            <For each={result().tools}>
              {(tool) => (
                <article
                  class="mcp-tool-card"
                  data-component="mcp-card"
                  data-testid={`mcp-tool-${tool.name}`}
                >
                  <div class="mcp-tool-copy">
                    <code>{tool.name}</code>
                    <span>
                      {tool.description ?? props.copy("服务未提供描述", "No description")}
                    </span>
                  </div>
                </article>
              )}
            </For>
          </>
        )}
      </Show>
      <div class="dialog-actions">
        <Button disabled={props.busy} onClick={props.onCancel}>
          {props.copy("取消", "Cancel")}
        </Button>
        <Button
          data-testid="mcp-test-new-connection"
          disabled={props.busy || !props.connectorEnabled || !draftValid(props.draft)}
          onClick={props.onTest}
        >
          {props.copy("测试连接", "Test connection")}
        </Button>
        <Button
          data-testid="mcp-save-new-server"
          variant="primary"
          disabled={props.busy || !draftValid(props.draft)}
          onClick={props.onSave}
        >
          {props.busy ? props.copy("处理中…", "Working…") : props.copy("保存", "Save")}
        </Button>
      </div>
    </div>
  );
}

function ToolParameters(props: { tool: McpToolView }) {
  const i18n = useI18n();
  const copy = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const parameters = createMemo(() => schemaParameters(props.tool.inputSchema));
  return (
    <Show when={parameters().length > 0}>
      <div class="mcp-tool-parameters" aria-label={copy("Tool 参数", "Tool parameters")}>
        <For each={parameters()}>
          {(parameter) => (
            <div>
              <code>{parameter.name}</code>
              <span>{parameter.type}</span>
              <Show when={props.tool.requiredParameters.includes(parameter.name)}>
                <strong>{copy("必填", "Required")}</strong>
              </Show>
              <p>
                {parameter.description ||
                  copy("服务未提供参数描述", "No parameter description supplied")}
              </p>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}

function schemaParameters(schema: unknown): Array<{
  name: string;
  type: string;
  description: string;
}> {
  if (!isRecord(schema) || !isRecord(schema.properties)) return [];
  return Object.entries(schema.properties).map(([name, value]) => {
    if (!isRecord(value)) return { name, type: "unknown", description: "" };
    const rawType = value.type;
    const type = Array.isArray(rawType)
      ? rawType.filter((item): item is string => typeof item === "string").join(" | ")
      : typeof rawType === "string"
        ? rawType
        : value.anyOf || value.oneOf
          ? "composite"
          : "unknown";
    return {
      name,
      type,
      description: typeof value.description === "string" ? value.description : "",
    };
  });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function emptyDraft(): McpServerDraft {
  return {
    id: crypto.randomUUID(),
    displayName: "",
    enabled: false,
    transport: { kind: "streamable_http", url: "" },
    headers: [],
    readOnlyTools: [],
    startupTimeoutMs: 15_000,
    requestTimeoutMs: 60_000,
    maxMessageBytes: 1024 * 1024,
  };
}

function draftFromView(view: McpServerView): McpServerDraft {
  return {
    id: view.configuration.id,
    displayName: view.configuration.displayName,
    enabled: view.configuration.enabled,
    transport: view.configuration.transport,
    headers: view.configuration.headers.map((header) => ({
      name: header.name,
      value: header.secret ? null : header.value,
      secret: header.secret,
    })),
    readOnlyTools: view.configuration.readOnlyTools,
    startupTimeoutMs: view.configuration.startupTimeoutMs,
    requestTimeoutMs: view.configuration.requestTimeoutMs,
    maxMessageBytes: view.configuration.maxMessageBytes,
  };
}

function draftValid(draft: McpServerDraft): boolean {
  if (!draft.displayName.trim()) return false;
  if (draft.transport.kind === "streamable_http" && !draft.transport.url.trim()) return false;
  if (draft.transport.kind === "stdio" && !draft.transport.command.trim()) return false;
  return draft.headers.every(
    (header) => header.name.trim() && (header.value !== "" || header.value === null),
  );
}

function serverSubtitle(view: McpServerView): string {
  if (view.configuration.transport.kind === "streamable_http")
    return view.configuration.transport.url;
  return view.configuration.transport.command;
}
