import {
  CONTROL_PROTOCOL_VERSION,
  commandFailure,
  commands,
  type ApprovalRequestRecord,
  type BrowserObservation,
  type BrowserPermissionLedgerEntry,
  type BrowserPermissionRequest,
  type BrowserProfileKind,
  type BrowserSession,
  type ComputerFrame,
  type ComputerWindowIdentity,
  type FeatureFlags,
  type RunRecord,
  type RunRecoveryDecisionAction,
  type RunRecoverySnapshot,
  type SessionRecord,
  type WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  AlertTriangle,
  Badge,
  Button,
  Hand,
  Monitor,
  MousePointer2,
  PageHeading,
  RefreshCw,
  SearchField,
  SelectField,
  SettingsCard,
  ShieldCheck,
  StatusBanner,
  TextField,
} from "@hachimi/ui";
import { For, Show, createEffect, createMemo, createSignal, onMount } from "solid-js";
import { runMutationContext } from "./mutation-context";

type BrowserSurface = Pick<
  BrowserSession,
  "id" | "ownerRunId" | "origin" | "profileKind" | "revision" | "status"
>;

function latestRun(snapshot: WorkbenchSessionSnapshot | undefined): RunRecord | undefined {
  return snapshot?.runs[snapshot.runs.length - 1];
}

function isTerminalRun(status: RunRecord["status"]): boolean {
  return ["succeeded", "failed", "timed_out", "cancelled", "interrupted", "lost"].includes(status);
}

function timestamp(value: number | undefined, locale: string): string {
  if (!value) return "—";
  return new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(value);
}

function inferredBrowser(entry: BrowserPermissionLedgerEntry): BrowserSurface {
  return {
    id: entry.browserSessionId,
    ownerRunId: entry.ownerRunId,
    origin: entry.permission.origin,
    profileKind: "isolated",
    revision: entry.browserRevision,
    status: "ready",
  };
}

export function DesktopControlPage(props: {
  featureFlags: FeatureFlags;
  navigateHome: () => void;
}) {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const [sessions, setSessions] = createSignal<SessionRecord[]>([]);
  const [sessionQuery, setSessionQuery] = createSignal("");
  const [selectedSessionId, setSelectedSessionId] = createSignal<string>();
  const [snapshot, setSnapshot] = createSignal<WorkbenchSessionSnapshot>();
  const [recoveries, setRecoveries] = createSignal<RunRecoverySnapshot[]>([]);
  const [permissionRequests, setPermissionRequests] = createSignal<BrowserPermissionRequest[]>([]);
  const [permissionLedger, setPermissionLedger] = createSignal<BrowserPermissionLedgerEntry[]>([]);
  const [windows, setWindows] = createSignal<ComputerWindowIdentity[]>([]);
  const [selectedWindowHandle, setSelectedWindowHandle] = createSignal("");
  const [browserProfile, setBrowserProfile] = createSignal<BrowserProfileKind>("isolated");
  const [browserUrl, setBrowserUrl] = createSignal("https://example.com");
  const [browserSession, setBrowserSession] = createSignal<BrowserSurface>();
  const [browserObservation, setBrowserObservation] = createSignal<BrowserObservation>();
  const [computerFrame, setComputerFrame] = createSignal<ComputerFrame>();
  const [prompt, setPrompt] = createSignal(
    "保持观察优先；在执行任何 Browser 或 Computer 动作前说明目标并遵守审批。",
  );
  const [busy, setBusy] = createSignal<string>();
  const [failure, setFailure] = createSignal<string>();
  const [notice, setNotice] = createSignal<string>();

  const selectedRun = createMemo(() => latestRun(snapshot()));
  const activeRun = createMemo(() => {
    const run = selectedRun();
    return run && !isTerminalRun(run.status) ? run : undefined;
  });
  const filteredSessions = createMemo(() => {
    const query = sessionQuery().trim().toLocaleLowerCase();
    return sessions().filter(
      (session) => !query || session.title.toLocaleLowerCase().includes(query),
    );
  });
  const selectedRecoveries = createMemo(() =>
    recoveries().filter((entry) => entry.recovery.sessionId === selectedSessionId()),
  );
  const selectedPermissionRequests = createMemo(() =>
    permissionRequests().filter(
      (entry) => entry.ownerSessionId === selectedSessionId() && entry.status === "pending",
    ),
  );
  const selectedWindow = createMemo(() =>
    windows().find((window) => window.windowHandle === selectedWindowHandle()),
  );

  async function runBusy<T>(key: string, operation: () => Promise<T>): Promise<T | undefined> {
    setBusy(key);
    setFailure(undefined);
    setNotice(undefined);
    try {
      return await operation();
    } catch (error) {
      setFailure(commandFailure(error).message);
      return undefined;
    } finally {
      setBusy(undefined);
    }
  }

  async function refreshInventory() {
    const [nextSessions, nextRecoveries, hostSettings, permissions, requests, windowResponse] =
      await Promise.all([
        commands.listWorkbenchSessions(),
        props.featureFlags.runtimeFeatures.runRecovery
          ? commands.listRunRecoveries()
          : Promise.resolve([]),
        props.featureFlags.browserControl
          ? commands.localHostCommand({ kind: "browser_get_host_settings" })
          : Promise.resolve(undefined),
        props.featureFlags.browserControl
          ? commands.localHostCommand({ kind: "browser_list_permissions" })
          : Promise.resolve(undefined),
        props.featureFlags.browserControl
          ? commands.localHostCommand({ kind: "browser_list_permission_requests" })
          : Promise.resolve(undefined),
        props.featureFlags.computerObserve
          ? commands.localHostCommand({ kind: "computer_list_windows" })
          : Promise.resolve(undefined),
      ]);
    const desktopSessions = nextSessions.filter(
      (session) => session.entryProfile === "desktop_control" && !session.archived,
    );
    setSessions(desktopSessions);
    setRecoveries(nextRecoveries);
    if (hostSettings?.kind === "browser_host_settings") {
      setBrowserProfile(hostSettings.value.preferredProfileKind);
    }
    if (permissions?.kind === "browser_permissions") setPermissionLedger(permissions.value);
    if (requests?.kind === "browser_permission_requests") {
      setPermissionRequests(requests.value);
    }
    if (windowResponse?.kind === "computer_windows") {
      setWindows(windowResponse.value);
      setSelectedWindowHandle((current) =>
        windowResponse.value.some((window) => window.windowHandle === current)
          ? current
          : (windowResponse.value[0]?.windowHandle ?? ""),
      );
    }
    const selected = selectedSessionId();
    if (!selected && desktopSessions[0]) setSelectedSessionId(desktopSessions[0].id);
    if (selected && !desktopSessions.some((session) => session.id === selected)) {
      setSelectedSessionId(desktopSessions[0]?.id);
    }
  }

  async function loadSession(sessionId: string) {
    await commands.resumeAgentSession({
      sessionId,
      metadataOnly: true,
      transcriptBeforeSequence: null,
      transcriptLimit: 0,
    });
    const next = await commands.getWorkbenchSession(sessionId);
    setSnapshot(next);
    const restored = permissionLedger().find((entry) => entry.ownerSessionId === sessionId);
    setBrowserSession(restored ? inferredBrowser(restored) : undefined);
    setBrowserObservation(undefined);
    setComputerFrame(undefined);
  }

  onMount(() => {
    // eslint-disable-next-line solid/reactivity -- runBusy invokes this callback synchronously inside the tracked onMount scope.
    void runBusy("refresh", async () => {
      await commands.initializeAgentControl({
        clientVersion: "hachimi-desktop/0.2.1",
        protocolVersion: CONTROL_PROTOCOL_VERSION,
        supportedFeatures: [
          "session_lifecycle_v2",
          "event_resume",
          "desktop_control",
          "browser_control",
          "computer_control",
        ],
        experimentalFeatures: [],
      });
      await refreshInventory();
    });
  });

  createEffect(() => {
    const sessionId = selectedSessionId();
    if (!sessionId) {
      setSnapshot(undefined);
      return;
    }
    // eslint-disable-next-line solid/reactivity -- runBusy invokes this callback synchronously inside the tracked effect.
    void runBusy("session", () => loadSession(sessionId));
  });

  async function createSession(continueSelected: boolean) {
    const text = prompt().trim();
    if (!text) return;
    // eslint-disable-next-line solid/reactivity -- createSession is called only from a JSX event handler and runBusy invokes immediately.
    await runBusy("create", async () => {
      const result = await commands.startWorkbenchTask({
        idempotencyKey: crypto.randomUUID(),
        entryProfile: "desktop_control",
        sessionId: continueSelected ? (selectedSessionId() ?? null) : null,
        projectId: null,
        prompt: text,
        executionTarget: null,
        behaviorMode: "default",
        approvalPolicy: "only_when_needed",
        attachmentIds: [],
        skillIds: [],
      });
      setSelectedSessionId(result.session.id);
      setSnapshot(await commands.getWorkbenchSession(result.session.id));
      await refreshInventory();
      setNotice(zh() ? "DesktopControl 会话已启动。" : "DesktopControl session started.");
    });
  }

  async function startBrowser() {
    const current = snapshot();
    const run = activeRun();
    if (!current || !run) return;
    const url = browserUrl().trim();
    // eslint-disable-next-line solid/reactivity -- startBrowser is a JSX event handler and runBusy invokes immediately.
    await runBusy("browser-start", async () => {
      const settings = await commands.localHostCommand({ kind: "browser_get_host_settings" });
      const pairingId =
        browserProfile() === "chrome_extension" && settings.kind === "browser_host_settings"
          ? (settings.value.latestPairing?.id ?? null)
          : null;
      const response = await commands.localHostCommand({
        kind: "browser_start",
        session_id: current.session.id,
        run_id: run.id,
        profile_kind: browserProfile(),
        initial_url: url,
        pairing_id: pairingId,
      });
      if (response.kind !== "browser_session") throw new Error("browser_start_protocol_mismatch");
      const started = response.value;
      const origin = new URL(url).origin;
      const permission = await commands.localHostCommand({
        kind: "browser_grant_site_permission",
        context: runMutationContext(run),
        session_id: current.session.id,
        run_id: run.id,
        browser_session_id: started.id,
        expected_revision: started.revision,
        origin,
        capabilities: ["observe", "act", "upload", "download", "cookie_storage", "cdp"],
        decision: "allow_session",
        network_kind: "document",
        allow_private_network: false,
        expires_at_ms: null,
      });
      if (permission.kind !== "browser_permission") {
        throw new Error("browser_permission_protocol_mismatch");
      }
      setBrowserSession(started);
      await observeBrowser(started);
      await refreshInventory();
      setNotice(
        zh()
          ? "Browser Session 已启动并授权当前 Origin。"
          : "Browser Session started for the current origin.",
      );
    });
  }

  async function observeBrowser(candidate: BrowserSurface | undefined) {
    if (!candidate) return;
    const response = await commands.localHostCommand({
      kind: "browser_observe",
      browser_session_id: candidate.id,
      run_id: candidate.ownerRunId,
    });
    if (response.kind !== "browser_observation")
      throw new Error("browser_observe_protocol_mismatch");
    setBrowserObservation(response.value);
    setBrowserSession((current) =>
      current
        ? { ...current, revision: response.value.browserRevision, origin: response.value.origin }
        : current,
    );
  }

  async function observeCurrentBrowser() {
    const candidate = browserSession();
    if (!candidate) return;
    await runBusy("browser-observe", () => observeBrowser(candidate));
  }

  async function releaseBrowser(stop: boolean) {
    const candidate = browserSession();
    if (!candidate) return;
    // eslint-disable-next-line solid/reactivity -- releaseBrowser is a JSX event handler and runBusy invokes immediately.
    await runBusy(stop ? "browser-stop" : "browser-takeover", async () => {
      const response = await commands.localHostCommand({
        kind: stop ? "browser_stop" : "browser_take_over",
        browser_session_id: candidate.id,
        run_id: candidate.ownerRunId,
      });
      if (response.kind !== "browser_session") throw new Error("browser_release_protocol_mismatch");
      setBrowserSession(undefined);
      setBrowserObservation(undefined);
      await refreshInventory();
      setNotice(
        stop
          ? zh()
            ? "Browser Session 已停止。"
            : "Browser Session stopped."
          : zh()
            ? "浏览器已交还用户。"
            : "Browser control returned to the user.",
      );
    });
  }

  async function resolveBrowserPermission(request: BrowserPermissionRequest, allow: boolean) {
    const current = snapshot();
    const run = current?.runs.find((candidate) => candidate.id === request.ownerRunId);
    if (!current || !run) return;
    // eslint-disable-next-line solid/reactivity -- permission resolution is a JSX event handler and runBusy invokes immediately.
    await runBusy(`permission-${request.id}`, async () => {
      await commands.localHostCommand({
        kind: "browser_grant_site_permission",
        context: runMutationContext(run),
        session_id: request.ownerSessionId,
        run_id: request.ownerRunId,
        browser_session_id: request.browserSessionId,
        expected_revision: request.expectedBrowserRevision,
        origin: request.origin,
        capabilities: request.capabilities,
        decision: allow ? "allow_session" : "deny",
        network_kind: request.networkKind,
        allow_private_network: allow && request.privateNetwork,
        expires_at_ms: allow ? request.expiresAtMs : null,
      });
      await refreshInventory();
    });
  }

  async function observeComputer() {
    const current = snapshot();
    const run = activeRun();
    const target = selectedWindow();
    if (!current || !run || !target) return;
    // eslint-disable-next-line solid/reactivity -- observeComputer is a JSX event handler and runBusy invokes immediately.
    await runBusy("computer-observe", async () => {
      await commands.localHostCommand({
        kind: "computer_set_app_rule",
        session_id: current.session.id,
        rule: {
          appId: target.appId,
          observe: true,
          act: props.featureFlags.computerControl,
          alwaysAllowed: false,
          grantedBy: "",
          updatedAtMs: 0,
        },
      });
      const response = await commands.localHostCommand({
        kind: "computer_observe",
        session_id: current.session.id,
        run_id: run.id,
        window_handle: target.windowHandle,
      });
      if (response.kind !== "computer_frame") throw new Error("computer_observe_protocol_mismatch");
      setComputerFrame(response.value);
      setNotice(
        zh()
          ? "已创建临时观察帧；截图 token 不会写入普通数据库。"
          : "Ephemeral observation frame created; its screenshot token is not stored in the regular database.",
      );
    });
  }

  async function takeOverComputer() {
    const sessionId = selectedSessionId();
    if (!sessionId) return;
    await runBusy("computer-takeover", async () => {
      const response = await commands.localHostCommand({
        kind: "computer_take_over",
        session_id: sessionId,
      });
      if (response.kind !== "computer_taken_over") {
        throw new Error("computer_takeover_protocol_mismatch");
      }
      setComputerFrame(undefined);
      setNotice(
        zh()
          ? "用户已接管，旧 frame 与 input epoch 已失效。"
          : "User takeover invalidated the old frame and input epoch.",
      );
    });
  }

  async function resolveApproval(approval: ApprovalRequestRecord, approved: boolean) {
    // eslint-disable-next-line solid/reactivity -- approval resolution is a JSX event handler and runBusy invokes immediately.
    await runBusy(`approval-${approval.id}`, async () => {
      await commands.resolveWorkbenchApproval({
        approvalId: approval.id,
        decision: approved ? "approved" : "denied",
        expectedRunId: approval.runId,
        expectedGeneration: approval.runGeneration,
      });
      if (selectedSessionId())
        setSnapshot(await commands.getWorkbenchSession(selectedSessionId()!));
    });
  }

  async function resolveRecovery(recovery: RunRecoverySnapshot, action: RunRecoveryDecisionAction) {
    // eslint-disable-next-line solid/reactivity -- recovery resolution is a JSX event handler and runBusy invokes immediately.
    await runBusy(`recovery-${recovery.recovery.id}`, async () => {
      await commands.resolveRunRecovery({
        context: runMutationContext({
          id: recovery.recovery.runId,
          generation: recovery.recovery.interruptedGeneration,
        }),
        recoveryId: recovery.recovery.id,
        expectedRunId: recovery.recovery.runId,
        expectedInterruptedGeneration: recovery.recovery.interruptedGeneration,
        action,
      });
      await refreshInventory();
      if (selectedSessionId()) await loadSession(selectedSessionId()!);
    });
  }

  return (
    <main class="desktop-control-page" data-testid="desktop-control-page">
      <PageHeading
        eyebrow="DesktopControl"
        title={zh() ? "桌面控制中心" : "Desktop Control Center"}
        description={
          zh()
            ? "先观察、再授权、后行动。Browser 与 Computer 复用唯一 Agent Runtime。"
            : "Observe first, authorize explicitly, then act through the single Agent Runtime."
        }
        badge={selectedRun()?.status ?? (zh() ? "未启动" : "Not started")}
        badgeTone={selectedRun()?.status === "failed" ? "danger" : "info"}
        actions={
          <div class="desktop-control-heading-actions">
            <Button onClick={props.navigateHome}>
              {zh() ? "返回工作台" : "Back to Workbench"}
            </Button>
            <Button
              disabled={Boolean(busy())}
              onClick={() => void runBusy("refresh", refreshInventory)}
            >
              <RefreshCw size={15} /> {zh() ? "刷新" : "Refresh"}
            </Button>
          </div>
        }
      />

      <Show when={failure()}>
        {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
      </Show>
      <Show when={notice()}>
        {(message) => <StatusBanner tone="success">{message()}</StatusBanner>}
      </Show>
      <Show when={selectedRun() && !activeRun()}>
        <StatusBanner tone="warning">
          {zh()
            ? "当前 Run 已结束。Browser/Computer 的新授权不能沿用旧 generation；请点击“继续所选”创建 fresh Run。"
            : "The current Run is terminal. New Browser/Computer authority cannot reuse its generation; select Continue to create a fresh Run."}
        </StatusBanner>
      </Show>

      <div class="desktop-control-layout">
        <aside class="desktop-control-sessions">
          <SearchField
            label={zh() ? "搜索会话" : "Search sessions"}
            value={sessionQuery()}
            onInput={(event) => setSessionQuery(event.currentTarget.value)}
          />
          <div class="desktop-control-session-list">
            <For each={filteredSessions()}>
              {(session) => (
                <Button
                  classList={{ selected: selectedSessionId() === session.id }}
                  onClick={() => setSelectedSessionId(session.id)}
                >
                  <Monitor size={16} />
                  <span>
                    <strong>{session.title}</strong>
                    <small>{timestamp(session.updatedAtMs, i18n.locale())}</small>
                  </span>
                </Button>
              )}
            </For>
            <Show when={!filteredSessions().length}>
              <p>{zh() ? "暂无 DesktopControl 会话" : "No DesktopControl sessions"}</p>
            </Show>
          </div>
          <TextField
            label={zh() ? "会话目标" : "Session objective"}
            value={prompt()}
            maxLength={2_000}
            onInput={(event) => setPrompt(event.currentTarget.value)}
          />
          <div class="desktop-control-session-actions">
            <Button
              variant="primary"
              disabled={busy() === "create" || !prompt().trim()}
              onClick={() => void createSession(false)}
            >
              {zh() ? "新建会话" : "New session"}
            </Button>
            <Button
              disabled={!selectedSessionId() || busy() === "create"}
              onClick={() => void createSession(true)}
            >
              {zh() ? "继续所选" : "Continue selected"}
            </Button>
          </div>
        </aside>

        <section class="desktop-control-workspace">
          <SettingsCard class="desktop-control-card observe-card">
            <header>
              <span>
                <ShieldCheck size={18} /> Observe first
              </span>
              <Badge tone={computerFrame() || browserObservation() ? "success" : "neutral"}>
                {computerFrame() || browserObservation()
                  ? zh()
                    ? "观察有效"
                    : "Observation live"
                  : zh()
                    ? "等待观察"
                    : "Awaiting observation"}
              </Badge>
            </header>
            <div class="observe-summary">
              <div>
                <small>{zh() ? "当前会话" : "Session"}</small>
                <strong>{snapshot()?.session.title ?? "—"}</strong>
              </div>
              <div>
                <small>{zh() ? "当前应用 / 窗口" : "App / window"}</small>
                <strong>
                  {computerFrame()?.target.title ?? browserObservation()?.title ?? "—"}
                </strong>
              </div>
              <div>
                <small>{zh() ? "观察时间" : "Observed"}</small>
                <strong>
                  {timestamp(
                    computerFrame()?.createdAtMs ?? browserObservation()?.createdAtMs,
                    i18n.locale(),
                  )}
                </strong>
              </div>
              <div>
                <small>{zh() ? "授权范围" : "Authority"}</small>
                <strong>{selectedRun()?.configuration.permissionProfile ?? "—"}</strong>
              </div>
            </div>
          </SettingsCard>

          <div class="desktop-control-host-grid">
            <SettingsCard class="desktop-control-card">
              <header>
                <span>
                  <MousePointer2 size={18} /> Browser
                </span>
                <Badge tone={props.featureFlags.browserControl ? "info" : "danger"}>
                  {props.featureFlags.browserControl ? "enabled" : "disabled"}
                </Badge>
              </header>
              <SelectField
                label={zh() ? "Profile" : "Profile"}
                value={browserProfile()}
                options={[
                  { value: "isolated", label: zh() ? "隔离 Chromium" : "Isolated Chromium" },
                  { value: "chrome_extension", label: "Chrome Extension" },
                ]}
                disabled={Boolean(browserSession())}
                onChange={(value) => setBrowserProfile(value as BrowserProfileKind)}
              />
              <TextField
                label="URL"
                value={browserUrl()}
                onInput={(event) => setBrowserUrl(event.currentTarget.value)}
              />
              <div class="desktop-control-inline-actions">
                <Button
                  variant="primary"
                  disabled={
                    !activeRun() || Boolean(browserSession()) || !props.featureFlags.browserControl
                  }
                  onClick={() => void startBrowser()}
                >
                  {zh() ? "启动并授权" : "Start and authorize"}
                </Button>
                <Button disabled={!browserSession()} onClick={() => void observeCurrentBrowser()}>
                  {zh() ? "重新观察" : "Observe"}
                </Button>
                <Button disabled={!browserSession()} onClick={() => void releaseBrowser(false)}>
                  <Hand size={14} /> {zh() ? "用户接管" : "Take over"}
                </Button>
                <Button
                  tone="danger"
                  disabled={!browserSession()}
                  onClick={() => void releaseBrowser(true)}
                >
                  {zh() ? "停止" : "Stop"}
                </Button>
              </div>
              <dl class="desktop-control-details">
                <div>
                  <dt>Session</dt>
                  <dd>{browserSession()?.id ?? "—"}</dd>
                </div>
                <div>
                  <dt>Origin</dt>
                  <dd>{browserObservation()?.origin ?? browserSession()?.origin ?? "—"}</dd>
                </div>
                <div>
                  <dt>Revision</dt>
                  <dd>
                    {browserObservation()?.browserRevision ?? browserSession()?.revision ?? "—"}
                  </dd>
                </div>
                <div>
                  <dt>{zh() ? "页面" : "Page"}</dt>
                  <dd>{browserObservation()?.title ?? "—"}</dd>
                </div>
              </dl>
            </SettingsCard>

            <SettingsCard class="desktop-control-card">
              <header>
                <span>
                  <Monitor size={18} /> Computer
                </span>
                <Badge tone={props.featureFlags.computerObserve ? "info" : "danger"}>
                  {props.featureFlags.computerControl ? "interactive" : "observe only"}
                </Badge>
              </header>
              <SelectField
                label={zh() ? "应用窗口" : "Application window"}
                value={selectedWindowHandle()}
                options={windows().map((window) => ({
                  value: window.windowHandle,
                  label: `${window.appId} · ${window.title}`,
                }))}
                disabled={!windows().length}
                onChange={setSelectedWindowHandle}
              />
              <div class="desktop-control-inline-actions">
                <Button
                  variant="primary"
                  disabled={
                    !activeRun() || !selectedWindow() || !props.featureFlags.computerObserve
                  }
                  onClick={() => void observeComputer()}
                >
                  {zh() ? "授权并观察" : "Authorize and observe"}
                </Button>
                <Button disabled={!computerFrame()} onClick={() => void takeOverComputer()}>
                  <Hand size={14} /> {zh() ? "用户接管" : "Take over"}
                </Button>
              </div>
              <dl class="desktop-control-details">
                <div>
                  <dt>App</dt>
                  <dd>{computerFrame()?.target.appId ?? selectedWindow()?.appId ?? "—"}</dd>
                </div>
                <div>
                  <dt>Window</dt>
                  <dd>{computerFrame()?.target.title ?? selectedWindow()?.title ?? "—"}</dd>
                </div>
                <div>
                  <dt>Frame</dt>
                  <dd>{computerFrame()?.id ?? "—"}</dd>
                </div>
                <div>
                  <dt>Input epoch</dt>
                  <dd>{computerFrame()?.inputEpoch ?? "—"}</dd>
                </div>
              </dl>
              <Show when={computerFrame()}>
                {(frame) => (
                  <p class="desktop-control-fence">
                    {frame().width}×{frame().height} · fingerprint{" "}
                    {frame().target.fingerprint.slice(0, 12)}… · expires{" "}
                    {timestamp(frame().expiresAtMs, i18n.locale())}
                  </p>
                )}
              </Show>
            </SettingsCard>
          </div>

          <Show
            when={
              snapshot()?.pendingApprovals.length ||
              selectedPermissionRequests().length ||
              selectedRecoveries().length
            }
          >
            <SettingsCard class="desktop-control-card attention-card">
              <header>
                <span>
                  <AlertTriangle size={18} /> {zh() ? "需要处理" : "Needs attention"}
                </span>
                <Badge tone="warning">
                  {(snapshot()?.pendingApprovals.length ?? 0) +
                    selectedPermissionRequests().length +
                    selectedRecoveries().length}
                </Badge>
              </header>
              <For each={snapshot()?.pendingApprovals ?? []}>
                {(approval) => (
                  <article>
                    <div>
                      <strong>{approval.action}</strong>
                      <p>{approval.riskSummary}</p>
                      <small>
                        {approval.targetHost} · generation {approval.runGeneration}
                      </small>
                    </div>
                    <div>
                      <Button
                        disabled={busy() === `approval-${approval.id}`}
                        onClick={() => void resolveApproval(approval, false)}
                      >
                        {zh() ? "拒绝" : "Deny"}
                      </Button>
                      <Button
                        variant="primary"
                        disabled={busy() === `approval-${approval.id}`}
                        onClick={() => void resolveApproval(approval, true)}
                      >
                        {zh() ? "批准" : "Approve"}
                      </Button>
                    </div>
                  </article>
                )}
              </For>
              <For each={selectedPermissionRequests()}>
                {(request) => (
                  <article>
                    <div>
                      <strong>{request.origin}</strong>
                      <p>{request.capabilities.join(", ")}</p>
                      <small>
                        {request.networkKind} · expires{" "}
                        {timestamp(request.expiresAtMs, i18n.locale())}
                      </small>
                    </div>
                    <div>
                      <Button onClick={() => void resolveBrowserPermission(request, false)}>
                        {zh() ? "拒绝" : "Deny"}
                      </Button>
                      <Button
                        variant="primary"
                        onClick={() => void resolveBrowserPermission(request, true)}
                      >
                        {zh() ? "允许本会话" : "Allow session"}
                      </Button>
                    </div>
                  </article>
                )}
              </For>
              <For each={selectedRecoveries()}>
                {(entry) => (
                  <article>
                    <div>
                      <strong>{entry.recovery.reasonCode}</strong>
                      <p>
                        {entry.checkpoint?.phase ?? "checkpoint unavailable"} ·{" "}
                        {entry.checkpoint?.recoveryPolicy ?? "manual"}
                      </p>
                      <small>
                        generation {entry.recovery.interruptedGeneration} →{" "}
                        {entry.recovery.resumeGeneration}
                      </small>
                    </div>
                    <div>
                      <Button onClick={() => void resolveRecovery(entry, "abandon_run")}>
                        {zh() ? "放弃" : "Abandon"}
                      </Button>
                      <Button
                        variant="primary"
                        disabled={entry.checkpoint?.recoveryPolicy === "non_replayable"}
                        onClick={() => void resolveRecovery(entry, "resume_safe_remainder")}
                      >
                        {zh() ? "继续安全部分" : "Resume safe remainder"}
                      </Button>
                    </div>
                  </article>
                )}
              </For>
            </SettingsCard>
          </Show>
        </section>
      </div>
    </main>
  );
}
