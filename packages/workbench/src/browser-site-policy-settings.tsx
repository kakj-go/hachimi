import {
  commandFailure,
  commands,
  type BrowserCapability,
  type BrowserSitePolicy,
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
  TextField,
} from "@hachimi/ui";
import { For, Show, createSignal, onMount } from "solid-js";

const DEFAULT_CAPABILITIES: BrowserCapability[] = ["observe", "act"];

function isLikelyPrivateOrigin(value: string) {
  try {
    const hostname = new URL(value.trim()).hostname.toLowerCase();
    if (
      hostname === "localhost" ||
      hostname.endsWith(".localhost") ||
      hostname.endsWith(".local") ||
      hostname === "::1" ||
      hostname === "127.0.0.1" ||
      hostname.startsWith("10.") ||
      hostname.startsWith("192.168.")
    ) {
      return true;
    }
    const octets = hostname.split(".").map(Number);
    const first = octets[0] ?? -1;
    const second = octets[1] ?? -1;
    return (
      octets.length === 4 &&
      octets.every((octet) => Number.isInteger(octet) && octet >= 0 && octet <= 255) &&
      first === 172 &&
      second >= 16 &&
      second <= 31
    );
  } catch {
    return false;
  }
}

function policyOptions(zh: () => boolean, privateNetwork: boolean) {
  const options = [
    { value: "ask", label: zh() ? "每次询问" : "Ask" },
    { value: "allow", label: zh() ? "始终允许" : "Allow" },
    { value: "block", label: zh() ? "始终阻止" : "Block" },
  ];
  return privateNetwork ? options.filter((option) => option.value !== "allow") : options;
}

export function BrowserSitePolicySettings() {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const [policies, setPolicies] = createSignal<BrowserSitePolicy[]>([]);
  const [origin, setOrigin] = createSignal("");
  const [decision, setDecision] = createSignal<HostPolicyDecision>("ask");
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();

  async function load() {
    setBusy(true);
    setFailure(undefined);
    try {
      setPolicies(await commands.listBrowserSitePolicies());
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function save() {
    if (!origin().trim()) return;
    setBusy(true);
    setFailure(undefined);
    try {
      const saved = await commands.updateBrowserSitePolicy({
        origin: origin().trim(),
        decision: decision(),
        capabilities: DEFAULT_CAPABILITIES,
        expectedRevision: null,
      });
      setPolicies((current) => [
        saved,
        ...current.filter((candidate) => candidate.origin !== saved.origin),
      ]);
      setOrigin("");
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function change(policy: BrowserSitePolicy, next: HostPolicyDecision) {
    setBusy(true);
    setFailure(undefined);
    try {
      const saved = await commands.updateBrowserSitePolicy({
        origin: policy.origin,
        decision: next,
        capabilities: policy.capabilities,
        expectedRevision: policy.revision,
      });
      setPolicies((current) =>
        current.map((candidate) => (candidate.origin === saved.origin ? saved : candidate)),
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function remove(policy: BrowserSitePolicy) {
    setBusy(true);
    setFailure(undefined);
    try {
      await commands.removeBrowserSitePolicy(policy.origin);
      setPolicies((current) => current.filter((candidate) => candidate.origin !== policy.origin));
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  onMount(() => void load());

  return (
    <SettingsSection title={zh() ? "网站访问" : "Website access"}>
      <Show when={failure()}>
        {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
      </Show>
      <SettingsCard>
        <SettingsRow
          label={zh() ? "添加精确 Origin" : "Add exact Origin"}
          description={
            zh()
              ? "策略同时应用于内置 CEF 和外置 Chrome 的 Agent 控制。"
              : "Policies apply to Agent control in both embedded CEF and external Chrome."
          }
        >
          <div class="host-domain-actions">
            <TextField
              label="Origin"
              value={origin()}
              placeholder="https://example.com"
              onInput={(event) => setOrigin(event.currentTarget.value)}
            />
            <Select
              label={zh() ? "默认决议" : "Default decision"}
              value={decision()}
              options={policyOptions(zh, isLikelyPrivateOrigin(origin()))}
              onChange={(value) => setDecision(value as HostPolicyDecision)}
            />
            <Button disabled={busy() || !origin().trim()} onClick={() => void save()}>
              {zh() ? "保存策略" : "Save policy"}
            </Button>
          </div>
        </SettingsRow>
        <For
          each={policies()}
          fallback={
            <SettingsRow label={zh() ? "持久站点策略" : "Persistent site policies"}>
              <span class="host-domain-empty">{zh() ? "暂无" : "None"}</span>
            </SettingsRow>
          }
        >
          {(policy) => (
            <SettingsRow
              label={policy.origin}
              description={`${policy.capabilities.join(", ")} · revision ${policy.revision}`}
            >
              <div class="host-domain-actions">
                <Show when={policy.privateNetwork}>
                  <Badge tone="warning">{zh() ? "私网" : "Private"}</Badge>
                </Show>
                <Select
                  label={zh() ? "策略" : "Policy"}
                  value={policy.decision}
                  options={policyOptions(zh, policy.privateNetwork)}
                  disabled={busy()}
                  onChange={(value) => void change(policy, value as HostPolicyDecision)}
                />
                <Button variant="danger" disabled={busy()} onClick={() => void remove(policy)}>
                  {zh() ? "移除" : "Remove"}
                </Button>
              </div>
            </SettingsRow>
          )}
        </For>
      </SettingsCard>
    </SettingsSection>
  );
}

export function PrivateBrowserSitePolicySettings() {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const [policies, setPolicies] = createSignal<BrowserSitePolicy[]>([]);
  const [origin, setOrigin] = createSignal("");
  const [decision, setDecision] = createSignal<HostPolicyDecision>("allow");
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();

  async function load() {
    setBusy(true);
    setFailure(undefined);
    try {
      setPolicies(
        (await commands.listBrowserSitePolicies()).filter((policy) => policy.privateNetwork),
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function save() {
    if (!origin().trim()) return;
    setBusy(true);
    setFailure(undefined);
    try {
      const saved = await commands.updatePrivateBrowserSitePolicy({
        origin: origin().trim(),
        decision: decision(),
        capabilities: DEFAULT_CAPABILITIES,
        expectedRevision: null,
      });
      setPolicies((current) => [
        saved,
        ...current.filter((candidate) => candidate.origin !== saved.origin),
      ]);
      setOrigin("");
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function change(policy: BrowserSitePolicy, next: HostPolicyDecision) {
    setBusy(true);
    setFailure(undefined);
    try {
      const saved = await commands.updatePrivateBrowserSitePolicy({
        origin: policy.origin,
        decision: next,
        capabilities: policy.capabilities,
        expectedRevision: policy.revision,
      });
      setPolicies((current) =>
        current.map((candidate) => (candidate.origin === saved.origin ? saved : candidate)),
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function remove(policy: BrowserSitePolicy) {
    setBusy(true);
    setFailure(undefined);
    try {
      await commands.removeBrowserSitePolicy(policy.origin);
      setPolicies((current) => current.filter((candidate) => candidate.origin !== policy.origin));
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }

  onMount(() => void load());

  return (
    <SettingsSection
      title={zh() ? "私网网站策略（开发者）" : "Private website policies (Developer)"}
    >
      <StatusBanner tone="warning">
        {zh()
          ? "私网 Origin 可访问本机或局域网服务。永久允许只在 Developer mode 下提供，且不会跳过上传、下载或其他副作用审批。"
          : "Private Origins can reach local or LAN services. Persistent Allow is available only in Developer mode and never bypasses upload, download, or other side-effect approval."}
      </StatusBanner>
      <Show when={failure()}>
        {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
      </Show>
      <SettingsCard>
        <SettingsRow
          label={zh() ? "添加私网精确 Origin" : "Add exact private Origin"}
          description={
            zh()
              ? "必须包含协议、主机和端口（如适用），路径会被规范化移除。"
              : "Include scheme, host, and port when applicable; paths are removed during normalization."
          }
        >
          <div class="host-domain-actions">
            <TextField
              label="Origin"
              value={origin()}
              placeholder="http://127.0.0.1:3000"
              onInput={(event) => setOrigin(event.currentTarget.value)}
            />
            <Select
              label={zh() ? "策略" : "Policy"}
              value={decision()}
              options={[
                { value: "ask", label: zh() ? "每次询问" : "Ask" },
                { value: "allow", label: zh() ? "始终允许" : "Allow" },
                { value: "block", label: zh() ? "始终阻止" : "Block" },
              ]}
              onChange={(value) => setDecision(value as HostPolicyDecision)}
            />
            <Button
              disabled={busy() || !origin().trim()}
              data-testid="private-browser-policy-save"
              onClick={() => void save()}
            >
              {zh() ? "保存策略" : "Save policy"}
            </Button>
          </div>
        </SettingsRow>
        <For
          each={policies()}
          fallback={
            <SettingsRow label={zh() ? "私网持久策略" : "Persistent private policies"}>
              <span class="host-domain-empty">{zh() ? "暂无" : "None"}</span>
            </SettingsRow>
          }
        >
          {(policy) => (
            <SettingsRow
              label={policy.origin}
              description={`${policy.capabilities.join(", ")} · revision ${policy.revision}`}
            >
              <div class="host-domain-actions">
                <Badge tone="warning">{zh() ? "私网" : "Private"}</Badge>
                <Select
                  label={zh() ? "策略" : "Policy"}
                  value={policy.decision}
                  options={[
                    { value: "ask", label: zh() ? "询问" : "Ask" },
                    { value: "allow", label: zh() ? "允许" : "Allow" },
                    { value: "block", label: zh() ? "阻止" : "Block" },
                  ]}
                  disabled={busy()}
                  onChange={(value) => void change(policy, value as HostPolicyDecision)}
                />
                <Button variant="danger" disabled={busy()} onClick={() => void remove(policy)}>
                  {zh() ? "移除" : "Remove"}
                </Button>
              </div>
            </SettingsRow>
          )}
        </For>
      </SettingsCard>
    </SettingsSection>
  );
}
