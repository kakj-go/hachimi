import {
  commandFailure,
  type BrowserWorkspace,
  type BrowserWorkspaceMutation,
  type BrowserPermissionDecision,
  type EmbeddedBrowserPermissionRequest,
  type BrowserAutomationLease,
  type BrowserHostSettings,
  type HostAccessDecision,
  type HostAccessRequestRecord,
  type WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import {
  ArrowLeft,
  ArrowRight,
  Button,
  ExternalLink,
  Globe,
  Hand,
  Plus,
  Play,
  RefreshCw,
  Send,
  ShieldAlert,
  ShieldCheck,
  Square,
  TextField,
  X,
} from "@hachimi/ui";
import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  untrack,
} from "solid-js";

import type { BrowserShortcutRequested, WorkbenchCommandPort } from "../workbench-command-port";

type BrowserHostAccessRequest = HostAccessRequestRecord & {
  target: Extract<HostAccessRequestRecord["target"], { kind: "browser" }>;
};

function isBrowserHostAccessRequest(
  request: HostAccessRequestRecord,
): request is BrowserHostAccessRequest {
  return request.target.kind === "browser";
}

function mutationKey() {
  return globalThis.crypto?.randomUUID?.() ?? `browser-${Date.now()}-${Math.random()}`;
}

function tabLabel(tab: BrowserWorkspace["tabs"][number], zh: boolean) {
  if (tab.title.trim()) return tab.title;
  if (tab.url === "about:blank") return zh ? "新标签页" : "New tab";
  try {
    return new URL(tab.url).hostname || tab.url;
  } catch {
    return tab.url;
  }
}

function surfaceBlocked() {
  return Boolean(
    document.querySelector('[role="dialog"], dialog[open], [data-native-surface-blocker="true"]'),
  );
}

function ExternalBrowserInspector(props: {
  snapshot: WorkbenchSessionSnapshot;
  commandPort: WorkbenchCommandPort;
  locale: "zh-CN" | "en-US";
  leaseId?: string;
  browserSessionId?: string;
}) {
  const zh = () => props.locale === "zh-CN";
  const initialLease = () =>
    props.snapshot.browserAutomationLeases.find(
      (candidate) =>
        candidate.surface === "external_chrome" &&
        (!props.leaseId || candidate.id === props.leaseId),
    );
  const [lease, setLease] = createSignal<BrowserAutomationLease | undefined>(initialLease());
  const [settings, setSettings] = createSignal<BrowserHostSettings>();
  const [requests, setRequests] = createSignal<HostAccessRequestRecord[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  const observation = createMemo(() => {
    const current = lease();
    return current
      ? props.snapshot.externalBrowserObservations.find(
          (candidate) => candidate.leaseId === current.id,
        )?.observation
      : undefined;
  });
  const browserSession = createMemo(() => {
    const observedSessionId = observation()?.browserSessionId ?? props.browserSessionId;
    return props.snapshot.browserSessions.find((candidate) => candidate.id === observedSessionId);
  });
  const pendingRequest = createMemo(() =>
    requests().find(
      (request): request is BrowserHostAccessRequest =>
        request.status === "pending" &&
        isBrowserHostAccessRequest(request) &&
        request.target.surface === "external_chrome",
    ),
  );
  const screenshot = createMemo(() => {
    const current = observation();
    return current?.screenshotBase64 && current.screenshotMimeType
      ? `data:${current.screenshotMimeType};base64,${current.screenshotBase64}`
      : undefined;
  });

  async function resolveAccess(decision: HostAccessDecision) {
    const request = pendingRequest();
    if (!request) return;
    setBusy(true);
    setFailure(undefined);
    try {
      const resolved = await props.commandPort.resolveHostAccessRequest({
        requestId: request.id,
        decision,
      });
      setRequests((current) =>
        current.map((candidate) => (candidate.id === resolved.id ? resolved : candidate)),
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function changeControl(action: "take_over" | "resume" | "stop") {
    const current = lease();
    if (!current) return;
    setBusy(true);
    setFailure(undefined);
    try {
      if (action === "take_over") {
        setLease(await props.commandPort.takeOverBrowserAutomation(current.id));
      } else if (action === "resume") {
        setLease(await props.commandPort.resumeBrowserAutomation(current.id));
      } else {
        setLease(await props.commandPort.stopBrowserAutomation(current.id));
      }
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  onMount(() => {
    void props.commandPort
      .getBrowserHostSettings()
      .then(setSettings)
      .catch((error) => setFailure(commandFailure(error).message));
    void props.commandPort
      .listHostAccessRequests(props.snapshot.session.id)
      .then(setRequests)
      .catch(() => undefined);
  });

  createEffect(() => {
    const next = initialLease();
    if (next) setLease(next);
    setRequests(props.snapshot.hostAccessRequests);
  });

  return (
    <div class="browser-inspector browser-external-inspector">
      <div class="browser-external-header">
        <div>
          <strong>
            {observation()?.title || browserSession()?.currentUrl || "External Chrome"}
          </strong>
          <span>{observation()?.url || browserSession()?.currentUrl}</span>
        </div>
        <span data-status={settings()?.latestPairing?.confirmed ? "healthy" : "unavailable"}>
          {settings()?.latestPairing?.confirmed
            ? zh()
              ? "扩展已配对"
              : "Extension paired"
            : zh()
              ? "扩展未配对"
              : "Extension unavailable"}
        </span>
      </div>
      <Show
        when={screenshot()}
        fallback={
          <div class="browser-external-empty">
            {zh() ? "等待 Agent 获取页面观察..." : "Waiting for an Agent observation..."}
          </div>
        }
      >
        {(source) => (
          <img
            class="browser-external-screenshot"
            src={source()}
            alt={observation()?.title || "External Chrome observation"}
          />
        )}
      </Show>
      <Show when={pendingRequest()}>
        {(request) => (
          <div class="browser-permission-prompt">
            <div>
              <ShieldAlert size={18} />
              <span>
                <strong>{zh() ? "Agent 请求访问网站" : "Agent requests site access"}</strong>
                <small>{request().target.origin}</small>
              </span>
            </div>
            <div class="browser-permission-actions">
              <Button disabled={busy()} onClick={() => void resolveAccess("allow_once")}>
                {zh() ? "允许一次" : "Allow once"}
              </Button>
              <Button disabled={busy()} onClick={() => void resolveAccess("allow_session")}>
                {zh() ? "本会话允许" : "Allow session"}
              </Button>
              <Button
                disabled={busy() || request().target.private_network}
                onClick={() => void resolveAccess("always_allow")}
              >
                {zh() ? "始终允许" : "Always allow"}
              </Button>
              <Button disabled={busy()} onClick={() => void resolveAccess("always_block")}>
                {zh() ? "始终阻止" : "Always block"}
              </Button>
              <Button variant="danger" disabled={busy()} onClick={() => void resolveAccess("deny")}>
                {zh() ? "拒绝" : "Deny"}
              </Button>
            </div>
          </div>
        )}
      </Show>
      <Show when={failure()}>
        {(message) => <p class="browser-inspector-error">{message()}</p>}
      </Show>
      <Show when={lease()}>
        {(current) => (
          <div class="browser-automation-control" data-status={current().status}>
            <span>{current().status}</span>
            <Show when={current().status === "active"}>
              <Button
                data-testid="external-browser-take-over"
                disabled={busy()}
                onClick={() => void changeControl("take_over")}
              >
                <Hand size={14} />
                {zh() ? "接管" : "Take over"}
              </Button>
            </Show>
            <Show when={current().status === "suspended"}>
              <Button
                data-testid="external-browser-resume-agent"
                disabled={busy()}
                onClick={() => void changeControl("resume")}
              >
                <Play size={14} />
                {zh() ? "恢复 Agent" : "Resume Agent"}
              </Button>
            </Show>
            <Show when={current().status === "active" || current().status === "suspended"}>
              <Button
                variant="danger"
                data-testid="external-browser-stop-agent"
                disabled={busy()}
                onClick={() => void changeControl("stop")}
              >
                <Square size={13} />
                {zh() ? "停止 Agent 控制" : "Stop Agent control"}
              </Button>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}

type BrowserInspectorProps = {
  snapshot: WorkbenchSessionSnapshot;
  commandPort: WorkbenchCommandPort;
  locale: "zh-CN" | "en-US";
  leaseId?: string;
  surfaceKind?: "embedded" | "external_chrome";
  browserTabId?: string;
  browserSessionId?: string;
  initialUrl?: string;
};

export function BrowserInspector(props: BrowserInspectorProps) {
  return (
    <Show
      when={props.surfaceKind === "external_chrome"}
      fallback={<EmbeddedBrowserInspector {...props} />}
    >
      <ExternalBrowserInspector
        snapshot={props.snapshot}
        commandPort={props.commandPort}
        locale={props.locale}
        {...(props.leaseId ? { leaseId: props.leaseId } : {})}
        {...(props.browserSessionId ? { browserSessionId: props.browserSessionId } : {})}
      />
    </Show>
  );
}

function EmbeddedBrowserInspector(props: BrowserInspectorProps) {
  const zh = () => props.locale === "zh-CN";
  const commandPort = untrack(() => props.commandPort);
  const sessionId = untrack(() => props.snapshot.session.id);
  const [workspace, setWorkspace] = createSignal<BrowserWorkspace>();
  const [address, setAddress] = createSignal(props.initialUrl ?? "");
  const [addressFocused, setAddressFocused] = createSignal(false);
  const [history, setHistory] = createSignal<Array<{ url: string; title: string }>>([]);
  const [permissionRequests, setPermissionRequests] = createSignal<
    EmbeddedBrowserPermissionRequest[]
  >([]);
  const [hostAccessRequests, setHostAccessRequests] = createSignal<HostAccessRequestRecord[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  const [runtimeFailure, setRuntimeFailure] = createSignal<string>();
  let surface: HTMLDivElement | undefined;
  let addressInput: HTMLInputElement | undefined;
  let layoutRevision = 0;
  let layoutFrame: number | undefined;
  let historyTimer: number | undefined;

  const activeTab = createMemo(() => {
    const current = workspace();
    return current?.tabs.find((tab) => tab.id === current.activeTabId);
  });
  const pendingPermission = createMemo(() =>
    permissionRequests().find((request) => request.status === "pending"),
  );
  const pendingHostAccess = createMemo(() =>
    hostAccessRequests().find(
      (request): request is BrowserHostAccessRequest =>
        request.status === "pending" &&
        isBrowserHostAccessRequest(request) &&
        request.target.surface === "embedded",
    ),
  );

  function acceptWorkspace(next: BrowserWorkspace) {
    if (next.ownerSessionId !== sessionId) return;
    setWorkspace((current) => (!current || next.revision >= current.revision ? next : current));
  }

  async function openWorkspace() {
    setBusy(true);
    setFailure(undefined);
    try {
      const next = await commandPort.openBrowserWorkspace(sessionId, props.initialUrl ?? null);
      acceptWorkspace(next);
      void commandPort
        .listEmbeddedBrowserPermissionRequests(sessionId)
        .then((requests) =>
          setPermissionRequests((current) => [
            ...requests,
            ...current.filter(
              (candidate) => !requests.some((request) => request.id === candidate.id),
            ),
          ]),
        )
        .catch(() => undefined);
      void commandPort
        .listHostAccessRequests(sessionId)
        .then(setHostAccessRequests)
        .catch(() => undefined);
      const requestedTab = props.browserTabId;
      if (
        requestedTab &&
        requestedTab !== next.activeTabId &&
        next.tabs.some((tab) => tab.id === requestedTab)
      ) {
        await mutate({ kind: "activate_tab", tab_id: requestedTab }, next);
      } else if (!requestedTab && props.initialUrl) {
        const matching = next.tabs.find((tab) => tab.url === props.initialUrl);
        if (matching && matching.id !== next.activeTabId) {
          await mutate({ kind: "activate_tab", tab_id: matching.id }, next);
        } else if (!matching) {
          await mutate({ kind: "new_tab", url: props.initialUrl }, next);
        }
      }
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
      scheduleLayout();
    }
  }

  async function mutate(action: BrowserWorkspaceMutation, base = workspace()) {
    if (!base) throw new Error("browser_workspace_missing");
    setBusy(true);
    setFailure(undefined);
    try {
      const next = await commandPort.mutateBrowserWorkspace({
        workspaceId: base.id,
        expectedRevision: base.revision,
        idempotencyKey: mutationKey(),
        mutation: action,
      });
      acceptWorkspace(next);
      return next;
    } catch (error) {
      const message = commandFailure(error).message;
      setFailure(message);
      throw error;
    } finally {
      setBusy(false);
      scheduleLayout();
    }
  }

  function currentBounds() {
    const rect = surface?.getBoundingClientRect();
    if (!rect) return { x: 0, y: 0, width: 1, height: 1 };
    const left = Math.max(0, Math.min(window.innerWidth, rect.left));
    const top = Math.max(0, Math.min(window.innerHeight, rect.top));
    const right = Math.max(left, Math.min(window.innerWidth, rect.right));
    const bottom = Math.max(top, Math.min(window.innerHeight, rect.bottom));
    return {
      x: Math.round(left),
      y: Math.round(top),
      width: Math.max(1, Math.round(right - left)),
      height: Math.max(1, Math.round(bottom - top)),
    };
  }

  function reportLayout(visible = true) {
    const current = workspace();
    const tab = activeTab();
    if (!current || !tab) return;
    layoutRevision += 1;
    const isVisible = visible && document.visibilityState !== "hidden" && !surfaceBlocked();
    void commandPort
      .updateBrowserSurfaceLayout({
        workspaceId: current.id,
        tabId: tab.id,
        bounds: { ...currentBounds(), scaleFactor: window.devicePixelRatio || 1 },
        visible: isVisible,
        layoutRevision,
      })
      .catch((error) => {
        if (isVisible) setFailure(commandFailure(error).message);
      });
  }

  function scheduleLayout() {
    if (layoutFrame !== undefined) cancelAnimationFrame(layoutFrame);
    layoutFrame = requestAnimationFrame(() => {
      layoutFrame = undefined;
      reportLayout();
    });
  }

  async function navigate() {
    const tab = activeTab();
    const value = address().trim();
    if (!tab || !value) return;
    try {
      await mutate({ kind: "navigate", tab_id: tab.id, url: value });
      setHistory([]);
      addressInput?.blur();
    } catch {
      // The stable command failure is displayed above the native surface.
    }
  }

  function control(kind: "back" | "forward" | "reload" | "stop") {
    const tab = activeTab();
    if (!tab) return;
    void mutate({ kind, tab_id: tab.id }).catch(() => undefined);
  }

  function newTab(url: string | null = null) {
    if (busy()) return;
    void mutate({ kind: "new_tab", url }).catch(() => undefined);
  }

  function closeTab(tabId: string) {
    if (busy()) return;
    void mutate({ kind: "close_tab", tab_id: tabId }).catch(() => undefined);
  }

  function activateTab(tabId: string) {
    if (busy() || tabId === workspace()?.activeTabId) return;
    void mutate({ kind: "activate_tab", tab_id: tabId }).catch(() => undefined);
  }

  function queryHistory(value: string) {
    if (historyTimer !== undefined) window.clearTimeout(historyTimer);
    historyTimer = window.setTimeout(() => {
      void commandPort
        .getBrowserHistory(value, 8)
        .then((entries) => setHistory(entries))
        .catch(() => setHistory([]));
    }, 120);
  }

  async function resolvePermission(decision: BrowserPermissionDecision) {
    const request = pendingPermission();
    if (!request) return;
    setBusy(true);
    setFailure(undefined);
    try {
      const resolved = await commandPort.resolveEmbeddedBrowserPermission({
        requestId: request.id,
        decision,
      });
      setPermissionRequests((current) =>
        current.map((candidate) => (candidate.id === resolved.id ? resolved : candidate)),
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
      scheduleLayout();
    }
  }

  async function resolveHostAccess(decision: HostAccessDecision) {
    const request = pendingHostAccess();
    if (!request) return;
    setBusy(true);
    setFailure(undefined);
    try {
      const resolved = await commandPort.resolveHostAccessRequest({
        requestId: request.id,
        decision,
      });
      setHostAccessRequests((current) =>
        current.map((candidate) => (candidate.id === resolved.id ? resolved : candidate)),
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
      scheduleLayout();
    }
  }

  async function stopAutomation() {
    const lease = workspace()?.automationLease;
    if (!lease) return;
    setBusy(true);
    setFailure(undefined);
    try {
      await commandPort.stopBrowserAutomation(lease.id);
      await openWorkspace();
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
      scheduleLayout();
    }
  }

  function keyboardShortcut(event: KeyboardEvent) {
    if (event.ctrlKey && event.key.toLowerCase() === "l") {
      event.preventDefault();
      addressInput?.focus();
      addressInput?.select();
    } else if (event.ctrlKey && event.key.toLowerCase() === "t") {
      event.preventDefault();
      newTab();
    } else if (event.ctrlKey && event.key.toLowerCase() === "w") {
      event.preventDefault();
      const tab = activeTab();
      if (tab) closeTab(tab.id);
    } else if (event.ctrlKey && event.key.toLowerCase() === "r") {
      event.preventDefault();
      control("reload");
    } else if (event.altKey && event.key === "ArrowLeft") {
      event.preventDefault();
      control("back");
    } else if (event.altKey && event.key === "ArrowRight") {
      event.preventDefault();
      control("forward");
    }
  }

  function nativeShortcut(event: BrowserShortcutRequested) {
    if (!workspace()?.tabs.some((tab) => tab.id === event.tab_id)) return;
    if (event.shortcut === "focus_address") {
      addressInput?.focus();
      addressInput?.select();
    } else if (event.shortcut === "new_tab") {
      newTab();
    } else if (event.shortcut === "close_tab") {
      closeTab(event.tab_id);
    } else {
      void mutate({ kind: event.shortcut, tab_id: event.tab_id }).catch(() => undefined);
    }
  }

  createEffect(() => {
    const tab = activeTab();
    if (tab && !addressFocused()) setAddress(tab.url === "about:blank" ? "" : tab.url);
    scheduleLayout();
  });

  createEffect(() => {
    setHostAccessRequests(props.snapshot.hostAccessRequests);
  });

  onMount(() => {
    void openWorkspace();
    const unlisteners: Array<() => void> = [];
    const browserStoppedLabel = untrack(() =>
      zh() ? "内置浏览器已停止" : "Embedded browser stopped",
    );
    void commandPort
      .onBrowserTabStateChange((next) => acceptWorkspace(next))
      .then((unlisten) => unlisteners.push(unlisten));
    void commandPort
      .onBrowserWorkspaceChange((event) => {
        if (event.ownerSessionId === sessionId && !untrack(workspace)) {
          void untrack(() => openWorkspace());
        }
      })
      .then((unlisten) => unlisteners.push(unlisten));
    void commandPort
      .onBrowserShortcutRequested(nativeShortcut)
      .then((unlisten) => unlisteners.push(unlisten));
    void commandPort
      .onBrowserPermissionRequired?.((event) => {
        if (event.request.ownerSessionId !== sessionId) return;
        setPermissionRequests((current) => [
          event.request,
          ...current.filter((candidate) => candidate.id !== event.request.id),
        ]);
        scheduleLayout();
      })
      ?.then((unlisten) => unlisteners.push(unlisten));
    void commandPort
      .onBrowserRuntimeCrash((event) => {
        setRuntimeFailure(event.message || browserStoppedLabel);
        untrack(() => reportLayout(false));
      })
      .then((unlisten) => unlisteners.push(unlisten));

    const resizeObserver =
      typeof ResizeObserver === "function" ? new ResizeObserver(scheduleLayout) : undefined;
    if (surface) resizeObserver?.observe(surface);
    const mutationObserver =
      typeof MutationObserver === "function" ? new MutationObserver(scheduleLayout) : undefined;
    mutationObserver?.observe(document.body, { childList: true, subtree: true, attributes: true });
    window.addEventListener("resize", scheduleLayout);
    window.addEventListener("scroll", scheduleLayout, true);
    window.addEventListener("keydown", keyboardShortcut);
    document.addEventListener("visibilitychange", scheduleLayout);
    onCleanup(() => {
      reportLayout(false);
      for (const unlisten of unlisteners) unlisten();
      resizeObserver?.disconnect();
      mutationObserver?.disconnect();
      window.removeEventListener("resize", scheduleLayout);
      window.removeEventListener("scroll", scheduleLayout, true);
      window.removeEventListener("keydown", keyboardShortcut);
      document.removeEventListener("visibilitychange", scheduleLayout);
      if (layoutFrame !== undefined) cancelAnimationFrame(layoutFrame);
      if (historyTimer !== undefined) window.clearTimeout(historyTimer);
    });
  });

  return (
    <div class="browser-inspector">
      <div class="browser-inspector-tabs">
        <div class="browser-inspector-tab-strip" role="tablist">
          <For each={workspace()?.tabs ?? []}>
            {(tab) => (
              <div
                class="browser-inspector-tab"
                classList={{ active: tab.id === workspace()?.activeTabId }}
                role="tab"
                aria-selected={tab.id === workspace()?.activeTabId}
              >
                <Button
                  class="browser-inspector-tab-select"
                  title={tabLabel(tab, zh())}
                  data-testid={`browser-tab-${tab.id}`}
                  disabled={busy()}
                  onClick={() => activateTab(tab.id)}
                >
                  <Globe size={14} />
                  <span>{tabLabel(tab, zh())}</span>
                </Button>
                <Button
                  class="browser-inspector-tab-close"
                  aria-label={zh() ? "关闭标签页" : "Close tab"}
                  title={zh() ? "关闭标签页" : "Close tab"}
                  data-testid={`browser-tab-close-${tab.id}`}
                  disabled={busy()}
                  onClick={() => closeTab(tab.id)}
                >
                  <X size={13} />
                </Button>
              </div>
            )}
          </For>
        </div>
        <Button
          class="browser-inspector-new-tab"
          aria-label={zh() ? "新建标签页" : "New tab"}
          title={zh() ? "新建标签页 (Ctrl+T)" : "New tab (Ctrl+T)"}
          data-testid="browser-new-tab"
          disabled={busy()}
          onClick={() => newTab()}
        >
          <Plus size={16} />
        </Button>
      </div>

      <div class="browser-inspector-address">
        <Button
          aria-label={zh() ? "后退" : "Back"}
          title={zh() ? "后退 (Alt+Left)" : "Back (Alt+Left)"}
          disabled={busy() || !activeTab()?.canGoBack}
          onClick={() => control("back")}
        >
          <ArrowLeft size={16} />
        </Button>
        <Button
          aria-label={zh() ? "前进" : "Forward"}
          title={zh() ? "前进 (Alt+Right)" : "Forward (Alt+Right)"}
          disabled={busy() || !activeTab()?.canGoForward}
          onClick={() => control("forward")}
        >
          <ArrowRight size={16} />
        </Button>
        <Button
          aria-label={activeTab()?.loading ? (zh() ? "停止" : "Stop") : zh() ? "刷新" : "Reload"}
          title={
            activeTab()?.loading
              ? zh()
                ? "停止加载"
                : "Stop loading"
              : zh()
                ? "刷新 (Ctrl+R)"
                : "Reload (Ctrl+R)"
          }
          disabled={!activeTab()}
          onClick={() => control(activeTab()?.loading ? "stop" : "reload")}
        >
          <Show when={activeTab()?.loading} fallback={<RefreshCw size={15} />}>
            <Square size={13} />
          </Show>
        </Button>
        <div class="browser-address-field">
          <Show
            when={activeTab()?.navigationError}
            fallback={
              activeTab()?.url.startsWith("https://") ? (
                <ShieldCheck class="browser-security-icon secure" size={15} />
              ) : (
                <Globe class="browser-security-icon" size={15} />
              )
            }
          >
            <ShieldAlert class="browser-security-icon danger" size={15} />
          </Show>
          <TextField
            label={zh() ? "网址或搜索" : "Address or search"}
            testId="browser-address"
            ref={(element) => (addressInput = element)}
            value={address()}
            placeholder={zh() ? "搜索或输入网址" : "Search or enter address"}
            onFocus={() => {
              setAddressFocused(true);
              queryHistory(address());
            }}
            onBlur={() => window.setTimeout(() => setAddressFocused(false), 120)}
            onInput={(event) => {
              setAddress(event.currentTarget.value);
              queryHistory(event.currentTarget.value);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void navigate();
              }
            }}
          />
          <Show when={addressFocused() && history().length > 0}>
            <div class="browser-address-suggestions" role="listbox">
              <For each={history()}>
                {(entry) => (
                  <Button
                    onClick={() => {
                      setAddress(entry.url);
                      void navigate();
                    }}
                  >
                    <Globe size={14} />
                    <span>
                      <strong>{entry.title || entry.url}</strong>
                      <small>{entry.url}</small>
                    </span>
                  </Button>
                )}
              </For>
            </div>
          </Show>
        </div>
        <Button
          class="browser-inspector-go"
          aria-label={zh() ? "前往" : "Go"}
          title={zh() ? "前往" : "Go"}
          disabled={busy() || !address().trim()}
          onClick={() => void navigate()}
        >
          <Send size={15} />
        </Button>
        <Button
          class="browser-open-system"
          aria-label={zh() ? "在系统浏览器打开" : "Open in system browser"}
          title={zh() ? "在系统浏览器打开" : "Open in system browser"}
          disabled={!activeTab() || activeTab()?.url === "about:blank"}
          onClick={() => {
            const tab = activeTab();
            if (tab)
              void commandPort
                .openSystemBrowser(tab.url)
                .catch((error) => setFailure(commandFailure(error).message));
          }}
        >
          <ExternalLink size={15} />
        </Button>
      </div>

      <Show when={pendingPermission()}>
        {(request) => (
          <div class="browser-permission-prompt" data-native-surface-blocker="true">
            <div>
              <ShieldAlert size={18} />
              <span>
                <strong>{zh() ? "Agent 请求访问此网站" : "Agent requests site access"}</strong>
                <small>{request().origin}</small>
              </span>
            </div>
            <p>
              {request().privateNetwork
                ? zh()
                  ? "该地址位于本机或私有网络。"
                  : "This address is on localhost or a private network."
                : zh()
                  ? "授权只影响 Agent；你的手动浏览不需要此权限。"
                  : "This permission applies only to Agent control, not manual browsing."}
            </p>
            <div class="browser-permission-actions">
              <Button
                data-testid="browser-permission-allow-once"
                disabled={busy()}
                onClick={() => void resolvePermission("allow_once")}
              >
                {zh() ? "允许一次" : "Allow once"}
              </Button>
              <Button
                data-testid="browser-permission-allow-session"
                disabled={busy()}
                onClick={() => void resolvePermission("allow_session")}
              >
                {zh() ? "本会话允许" : "Allow session"}
              </Button>
              <Button
                data-testid="browser-permission-allow-persisted"
                disabled={busy() || request().privateNetwork}
                onClick={() => void resolvePermission("allow_persisted")}
              >
                {zh() ? "始终允许" : "Always allow"}
              </Button>
              <Button
                data-testid="browser-permission-deny"
                variant="danger"
                disabled={busy()}
                onClick={() => void resolvePermission("deny")}
              >
                {zh() ? "拒绝" : "Deny"}
              </Button>
            </div>
          </div>
        )}
      </Show>

      <Show when={pendingHostAccess()}>
        {(request) => (
          <div class="browser-permission-prompt" data-native-surface-blocker="true">
            <div>
              <ShieldAlert size={18} />
              <span>
                <strong>{zh() ? "Agent 请求访问此网站" : "Agent requests site access"}</strong>
                <small>{request().target.origin}</small>
              </span>
            </div>
            <div class="browser-permission-actions">
              <Button disabled={busy()} onClick={() => void resolveHostAccess("allow_once")}>
                {zh() ? "允许一次" : "Allow once"}
              </Button>
              <Button disabled={busy()} onClick={() => void resolveHostAccess("allow_session")}>
                {zh() ? "本会话允许" : "Allow session"}
              </Button>
              <Button
                disabled={busy() || request().target.private_network}
                onClick={() => void resolveHostAccess("always_allow")}
              >
                {zh() ? "始终允许" : "Always allow"}
              </Button>
              <Button disabled={busy()} onClick={() => void resolveHostAccess("always_block")}>
                {zh() ? "始终阻止" : "Always block"}
              </Button>
              <Button
                variant="danger"
                disabled={busy()}
                onClick={() => void resolveHostAccess("deny")}
              >
                {zh() ? "拒绝" : "Deny"}
              </Button>
            </div>
          </div>
        )}
      </Show>

      <Show when={failure() ?? runtimeFailure()}>
        {(message) => <p class="browser-inspector-error">{message()}</p>}
      </Show>
      <Show when={workspace()?.automationLease}>
        {(lease) => (
          <div class="browser-automation-control" data-status={lease().status}>
            <span>
              {lease().status === "active"
                ? zh()
                  ? "Agent 正在控制此标签页"
                  : "Agent is controlling this tab"
                : zh()
                  ? "你正在控制此标签页"
                  : "You are controlling this tab"}
            </span>
            <Show
              when={lease().status === "active"}
              fallback={
                <Button
                  data-testid="browser-resume-agent"
                  disabled={busy()}
                  onClick={() => void mutate({ kind: "resume_automation" }).catch(() => undefined)}
                >
                  <Play size={14} />
                  {zh() ? "恢复 Agent" : "Resume agent"}
                </Button>
              }
            >
              <Button
                data-testid="browser-take-over"
                disabled={busy()}
                onClick={() => void mutate({ kind: "take_over_automation" }).catch(() => undefined)}
              >
                <Hand size={14} />
                {zh() ? "接管" : "Take over"}
              </Button>
            </Show>
            <Button
              variant="danger"
              data-testid="browser-stop-agent"
              disabled={busy()}
              onClick={() => void stopAutomation()}
            >
              <Square size={13} />
              {zh() ? "停止 Agent 控制" : "Stop Agent control"}
            </Button>
          </div>
        )}
      </Show>
      <div ref={surface} class="browser-native-surface" data-testid="browser-native-surface">
        <Show when={!workspace() && busy()}>
          <span>{zh() ? "正在启动浏览器..." : "Starting browser..."}</span>
        </Show>
      </div>
    </div>
  );
}
