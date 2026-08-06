import {
  commandFailure,
  commands,
  type ComputerAppCandidate,
  type ComputerAppDescriptor,
  type ComputerAppPolicy,
  type ComputerHostSettings,
  type HostAccessDecision,
  type HostAccessRequestRecord,
  type HostPolicyDecision,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  Badge,
  Button,
  Select,
  SettingsCard,
  SettingsRow,
  SettingsSection,
  StatusBanner,
  Switch as Toggle,
} from "@hachimi/ui";
import { For, Show, createEffect, createMemo, createSignal, on, onMount } from "solid-js";

import { RuntimeHealthBanner, runtimeErrorMessage } from "./runtime-health";

export function ComputerUseSettings(props: { refreshRevision?: number } = {}) {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const [settings, setSettings] = createSignal<ComputerHostSettings>();
  const [candidates, setCandidates] = createSignal<ComputerAppCandidate[]>([]);
  const [policies, setPolicies] = createSignal<ComputerAppPolicy[]>([]);
  const [requests, setRequests] = createSignal<HostAccessRequestRecord[]>([]);
  const [settingsLoading, setSettingsLoading] = createSignal(false);
  const [settingsSaving, setSettingsSaving] = createSignal(false);
  const [candidatesLoading, setCandidatesLoading] = createSignal(false);
  const [policiesLoading, setPoliciesLoading] = createSignal(false);
  const [policySavingIds, setPolicySavingIds] = createSignal<ReadonlySet<string>>(new Set());
  const [requestsLoading, setRequestsLoading] = createSignal(false);
  const [requestSavingIds, setRequestSavingIds] = createSignal<ReadonlySet<string>>(new Set());
  const [settingsFailure, setSettingsFailure] = createSignal<string>();
  const [candidatesFailure, setCandidatesFailure] = createSignal<string>();
  const [policiesFailure, setPoliciesFailure] = createSignal<string>();
  const [requestsFailure, setRequestsFailure] = createSignal<string>();
  const apps = createMemo(() => {
    const all = new Map(
      candidates().map((candidate) => [
        candidate.app.identityHash,
        { app: candidate.app, iconPngBase64: candidate.iconPngBase64 },
      ]),
    );
    for (const policy of policies()) {
      if (!all.has(policy.app.identityHash)) {
        all.set(policy.app.identityHash, { app: policy.app, iconPngBase64: null });
      }
    }
    return [...all.values()].sort((left, right) =>
      left.app.displayName.localeCompare(right.app.displayName),
    );
  });
  const pending = createMemo(() =>
    requests().filter(
      (request) => request.status === "pending" && request.target.kind === "computer",
    ),
  );

  async function loadSettings() {
    setSettingsLoading(true);
    setSettingsFailure(undefined);
    try {
      setSettings(await commands.getComputerHostSettings());
    } catch (error) {
      setSettingsFailure(commandFailure(error).message);
    } finally {
      setSettingsLoading(false);
    }
  }

  async function loadCandidates() {
    setCandidatesLoading(true);
    setCandidatesFailure(undefined);
    try {
      setCandidates(await commands.listComputerAppCandidates());
    } catch (error) {
      setCandidatesFailure(commandFailure(error).message);
    } finally {
      setCandidatesLoading(false);
    }
  }

  async function loadPolicies() {
    setPoliciesLoading(true);
    setPoliciesFailure(undefined);
    try {
      setPolicies(await commands.listComputerAppPolicies());
    } catch (error) {
      setPoliciesFailure(commandFailure(error).message);
    } finally {
      setPoliciesLoading(false);
    }
  }

  async function loadRequests() {
    setRequestsLoading(true);
    setRequestsFailure(undefined);
    try {
      setRequests(await commands.listHostAccessRequests());
    } catch (error) {
      setRequestsFailure(commandFailure(error).message);
    } finally {
      setRequestsLoading(false);
    }
  }

  function load() {
    void loadSettings();
    void loadCandidates();
    void loadPolicies();
    void loadRequests();
  }

  async function updateAutomation(automationEnabled: boolean) {
    setSettingsSaving(true);
    setSettingsFailure(undefined);
    try {
      setSettings(await commands.updateComputerHostSettings({ automationEnabled }));
    } catch (error) {
      setSettingsFailure(commandFailure(error).message);
    } finally {
      setSettingsSaving(false);
    }
  }

  async function updatePolicy(app: ComputerAppDescriptor, decision: HostPolicyDecision) {
    const current = policies().find((policy) => policy.app.identityHash === app.identityHash);
    setPolicySavingIds((entries) => new Set(entries).add(app.identityHash));
    setPoliciesFailure(undefined);
    try {
      const saved = await commands.updateComputerAppPolicy({
        identityHash: app.identityHash,
        decision,
        expectedRevision: current?.revision ?? null,
      });
      setPolicies((entries) => [
        saved,
        ...entries.filter((policy) => policy.app.identityHash !== saved.app.identityHash),
      ]);
    } catch (error) {
      setPoliciesFailure(commandFailure(error).message);
    } finally {
      setPolicySavingIds((entries) => {
        const next = new Set(entries);
        next.delete(app.identityHash);
        return next;
      });
    }
  }

  async function resolve(request: HostAccessRequestRecord, decision: HostAccessDecision) {
    setRequestSavingIds((entries) => new Set(entries).add(request.id));
    setRequestsFailure(undefined);
    try {
      const resolved = await commands.resolveHostAccessRequest({
        requestId: request.id,
        decision,
      });
      setRequests((entries) =>
        entries.map((entry) => (entry.id === resolved.id ? resolved : entry)),
      );
    } catch (error) {
      setRequestsFailure(commandFailure(error).message);
    } finally {
      setRequestSavingIds((entries) => {
        const next = new Set(entries);
        next.delete(request.id);
        return next;
      });
    }
  }

  onMount(load);
  createEffect(on(() => props.refreshRevision, load, { defer: true }));

  return (
    <>
      <SettingsSection title={zh() ? "Agent 自动化" : "Agent automation"}>
        <Show when={settingsFailure()}>
          {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
        </Show>
        <RuntimeHealthBanner component="computer_use" zh={zh()} />
        <SettingsCard>
          <SettingsRow
            label={zh() ? "运行环境" : "Runtime environment"}
            description={
              settings()?.runtimeHealth.errorCode
                ? runtimeErrorMessage(settings()!.runtimeHealth.errorCode, zh())
                : zh()
                  ? "窗口捕获与输入桌面可用；提权应用仍受 Windows 安全边界限制。"
                  : "Window capture and the input desktop are available; elevated apps remain protected by Windows."
            }
          >
            <Badge tone={settings()?.runtimeHealth.errorCode ? "warning" : "success"}>
              {settings()?.runtimeHealth.errorCode
                ? zh()
                  ? "已降级"
                  : "Degraded"
                : zh()
                  ? "可用"
                  : "Available"}
            </Badge>
          </SettingsRow>
          <SettingsRow
            label="Computer Use"
            description={
              zh()
                ? "Agent 自行解析应用和窗口；只有稳定应用身份有歧义时才会请求确认。"
                : "The Agent resolves applications and windows, asking only when stable app identity is ambiguous."
            }
          >
            <Toggle
              label={zh() ? "允许 Agent 控制本地应用" : "Allow Agent control of local apps"}
              testId="computer-automation-toggle"
              checked={settings()?.automationEnabled ?? false}
              disabled={settingsLoading() || settingsSaving() || !settings()}
              onChange={(enabled) => void updateAutomation(enabled)}
            />
          </SettingsRow>
        </SettingsCard>
      </SettingsSection>

      <Show when={pending().length > 0 || requestsFailure() || requestsLoading()}>
        <SettingsSection title={zh() ? "待处理访问" : "Pending access"}>
          <Show when={requestsFailure()}>
            {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
          </Show>
          <Show when={requestsLoading() && pending().length === 0}>
            <SettingsCard>
              <SettingsRow label={zh() ? "访问请求" : "Access requests"}>
                <Badge>{zh() ? "正在检查" : "Checking"}</Badge>
              </SettingsRow>
            </SettingsCard>
          </Show>
          <Show when={pending().length > 0}>
            <SettingsCard>
              <For each={pending()}>
                {(request) => (
                  <SettingsRow
                    label={
                      request.target.kind === "computer"
                        ? request.target.app.displayName
                        : "Computer"
                    }
                    description={request.capabilities.join(", ")}
                  >
                    <div class="host-domain-actions">
                      <Button
                        disabled={requestSavingIds().has(request.id)}
                        onClick={() => void resolve(request, "allow_once")}
                      >
                        {zh() ? "允许一次" : "Allow once"}
                      </Button>
                      <Button
                        disabled={requestSavingIds().has(request.id)}
                        onClick={() => void resolve(request, "allow_session")}
                      >
                        {zh() ? "本会话允许" : "Allow session"}
                      </Button>
                      <Button
                        disabled={requestSavingIds().has(request.id)}
                        onClick={() => void resolve(request, "always_allow")}
                      >
                        {zh() ? "始终允许" : "Always allow"}
                      </Button>
                      <Button
                        disabled={requestSavingIds().has(request.id)}
                        onClick={() => void resolve(request, "always_block")}
                      >
                        {zh() ? "始终阻止" : "Always block"}
                      </Button>
                      <Button
                        variant="danger"
                        disabled={requestSavingIds().has(request.id)}
                        onClick={() => void resolve(request, "deny")}
                      >
                        {zh() ? "拒绝" : "Deny"}
                      </Button>
                    </div>
                  </SettingsRow>
                )}
              </For>
            </SettingsCard>
          </Show>
        </SettingsSection>
      </Show>

      <SettingsSection title={zh() ? "应用访问策略" : "Application access policies"}>
        <Show when={candidatesFailure()}>
          {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
        </Show>
        <Show when={policiesFailure()}>
          {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
        </Show>
        <SettingsCard>
          <For
            each={apps()}
            fallback={
              <SettingsRow label={zh() ? "可用应用" : "Available applications"}>
                <Badge>
                  {candidatesLoading() || policiesLoading()
                    ? zh()
                      ? "正在发现"
                      : "Discovering"
                    : zh()
                      ? "暂无"
                      : "None"}
                </Badge>
              </SettingsRow>
            }
          >
            {(entry) => {
              const app = () => entry.app;
              const policy = () =>
                policies().find((policy) => policy.app.identityHash === app().identityHash);
              return (
                <SettingsRow
                  label={
                    <span
                      class="computer-app-label"
                      title={app().executablePath ?? app().executableName}
                    >
                      <span class="computer-app-icon" aria-hidden="true">
                        <Show when={entry.iconPngBase64} fallback={app().displayName.slice(0, 1)}>
                          {(icon) => (
                            <img src={`data:image/png;base64,${icon()}`} alt="" loading="lazy" />
                          )}
                        </Show>
                      </span>
                      <span>
                        <strong>{app().displayName}</strong>
                        <small>
                          {app().publisher ?? (zh() ? "发布者未知" : "Unknown publisher")}
                        </small>
                      </span>
                    </span>
                  }
                  description={app().executableName}
                >
                  <div title={app().executablePath ?? app().executableName}>
                    <Select
                      label={zh() ? "访问策略" : "Access policy"}
                      value={policy()?.decision ?? "ask"}
                      options={[
                        { value: "ask", label: zh() ? "询问" : "Ask" },
                        { value: "allow", label: zh() ? "允许" : "Allow" },
                        { value: "block", label: zh() ? "阻止" : "Block" },
                      ]}
                      disabled={policySavingIds().has(app().identityHash)}
                      onChange={(value) => void updatePolicy(app(), value as HostPolicyDecision)}
                    />
                  </div>
                </SettingsRow>
              );
            }}
          </For>
        </SettingsCard>
      </SettingsSection>
    </>
  );
}
