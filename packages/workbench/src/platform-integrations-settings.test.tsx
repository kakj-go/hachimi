import type {
  GatewayHealth,
  IntegrationProviderAccount,
  IntegrationProviderDefinition,
} from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import { For, Show, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { PlatformIntegrationsSettings } from "./platform-integrations-settings";

const commandMocks = vi.hoisted(() => ({
  listIntegrationProviders: vi.fn(),
  listEnterpriseIntegrations: vi.fn(),
  getGatewayHealth: vi.fn(),
  upsertEnterpriseIntegration: vi.fn(),
  setEnterpriseIntegrationCapabilities: vi.fn(),
  probeEnterpriseIntegration: vi.fn(),
  removeEnterpriseIntegration: vi.fn(),
  listChannelAuthorizations: vi.fn(),
  upsertChannelAuthorization: vi.fn(),
  getChannelAccessPolicy: vi.fn(),
  updateChannelAccessPolicy: vi.fn(),
  createChannelPairingCode: vi.fn(),
  createChannelIdentityLinkCode: vi.fn(),
  listChannelIdentityTransferPreviews: vi.fn(),
  transferChannelIdentity: vi.fn(),
  beginIlinkQrLogin: vi.fn(),
  pollIlinkQrLogin: vi.fn(),
  cancelIlinkQrLogin: vi.fn(),
}));

vi.mock("@hachimi/contracts", () => ({
  commands: commandMocks,
  commandFailure: (reason: unknown) => ({ code: "command_failed", message: String(reason) }),
}));

vi.mock("@hachimi/ui", () => ({
  Badge: (props: { children?: JSX.Element }) => <span>{props.children}</span>,
  Button: (props: {
    children?: JSX.Element;
    disabled?: boolean;
    "aria-label"?: string;
    onClick?: () => void;
  }) => (
    <button
      aria-label={props["aria-label"]}
      disabled={props.disabled}
      onClick={() => props.onClick?.()}
    >
      {props.children}
    </button>
  ),
  Checkbox: (props: {
    label: string;
    checked: boolean;
    disabled?: boolean;
    onChange?: (event: Event & { currentTarget: HTMLInputElement }) => void;
  }) => (
    <input
      aria-label={props.label}
      type="checkbox"
      checked={props.checked}
      disabled={props.disabled}
      onChange={(event) => props.onChange?.(event)}
    />
  ),
  FormField: (props: { label: string; children?: JSX.Element }) => (
    <label>
      <span>{props.label}</span>
      {props.children}
    </label>
  ),
  Dialog: (props: {
    open: boolean;
    title: string;
    description?: string;
    children?: JSX.Element;
    onOpenChange?: (open: boolean) => void;
  }) => (
    <Show when={props.open}>
      <section role="dialog" aria-label={props.title}>
        <h2>{props.title}</h2>
        <p>{props.description}</p>
        {props.children}
        <button aria-label="关闭" onClick={() => props.onOpenChange?.(false)} />
      </section>
    </Show>
  ),
  PageHeading: (props: { title: string; description?: string; actions?: JSX.Element }) => (
    <header>
      <h1>{props.title}</h1>
      <p>{props.description}</p>
      {props.actions}
    </header>
  ),
  PermissionPolicyEditor: (props: {
    value: { level: string } & Record<string, unknown>;
    testId?: string;
    disabled?: boolean;
    zh: boolean;
    onChange?: (value: { level: string } & Record<string, unknown>) => void;
  }) => (
    <label>
      <span>{props.zh ? "权限档位" : "Permission level"}</span>
      <select
        aria-label={props.zh ? "权限档位" : "Permission level"}
        data-testid={props.testId}
        disabled={props.disabled}
        value={props.value.level}
        onChange={(event) => props.onChange?.({ ...props.value, level: event.currentTarget.value })}
      >
        <option value="read_only">Read only</option>
        <option value="writable">Writable</option>
        <option value="full_access">Full access</option>
      </select>
    </label>
  ),
  SettingsCard: (props: { children?: JSX.Element }) => <div>{props.children}</div>,
  SettingsRow: (props: { label: string; description?: JSX.Element; children?: JSX.Element }) => (
    <section>
      <strong>{props.label}</strong>
      <span>{props.description}</span>
      {props.children}
    </section>
  ),
  SettingsSection: (props: { title: string; children?: JSX.Element }) => (
    <section aria-label={props.title}>{props.children}</section>
  ),
  SegmentedControl: (props: {
    label: string;
    value: string;
    options: { value: string; label: string }[];
    onChange?: (value: string) => void;
  }) => (
    <label>
      <span>{props.label}</span>
      <select
        aria-label={props.label}
        value={props.value}
        onChange={(event) => props.onChange?.(event.currentTarget.value)}
      >
        <For each={props.options}>
          {(option) => <option value={option.value}>{option.label}</option>}
        </For>
      </select>
    </label>
  ),
  StatusBanner: (props: { children?: JSX.Element }) => <div role="status">{props.children}</div>,
  Switch: (props: {
    label: string;
    checked: boolean;
    disabled?: boolean;
    onChange?: (checked: boolean) => void;
  }) => (
    <button
      role="switch"
      aria-label={props.label}
      aria-checked={props.checked}
      disabled={props.disabled}
      onClick={() => props.onChange?.(!props.checked)}
    />
  ),
  Tabs: (props: {
    value: string;
    tabs: { value: string; ariaLabel?: string; label: JSX.Element; content: JSX.Element }[];
    onChange?: (value: string) => void;
  }) => (
    <div>
      <div role="tablist">
        <For each={props.tabs}>
          {(tab) => (
            <button
              role="tab"
              aria-label={tab.ariaLabel}
              onClick={() => props.onChange?.(tab.value)}
            >
              {tab.label}
            </button>
          )}
        </For>
      </div>
      <For each={props.tabs}>
        {(tab) => <div hidden={tab.value !== props.value}>{tab.content}</div>}
      </For>
    </div>
  ),
  TextField: (props: {
    label: string;
    value: string;
    type?: string;
    onInput?: (event: InputEvent & { currentTarget: HTMLInputElement }) => void;
  }) => (
    <input
      aria-label={props.label}
      type={props.type ?? "text"}
      value={props.value}
      onInput={(event) => props.onInput?.(event)}
    />
  ),
  KeyRound: () => <span />,
  Link: () => <span />,
  NativeSelect: (props: {
    label: string;
    value: string;
    children?: JSX.Element;
    onChange?: (event: Event & { currentTarget: HTMLSelectElement }) => void;
  }) => (
    <label>
      <span>{props.label}</span>
      <select
        aria-label={props.label}
        value={props.value}
        onChange={(event) => props.onChange?.(event)}
      >
        {props.children}
      </select>
    </label>
  ),
  Plus: () => <span />,
  RefreshCw: () => <span />,
  ShieldCheck: () => <span />,
  Trash2: () => <span />,
  Users: () => <span />,
}));

const fields = {
  text: (id: string, label: string) => ({
    id,
    label,
    kind: "text" as const,
    required: true,
    capability: null,
  }),
  secret: (id: string, label: string) => ({
    id,
    label,
    kind: "secret" as const,
    required: true,
    capability: null,
  }),
};

const providers: IntegrationProviderDefinition[] = [
  {
    id: "dingtalk",
    nameZh: "钉钉",
    nameEn: "DingTalk",
    iconAsset: "integration-icons/dingtalk.svg",
    transport: "stream",
    authMethod: "client_secret",
    capabilities: ["api_access", "messaging", "dm", "group"],
    credentialFields: [
      fields.text("clientId", "Client/App ID"),
      fields.secret("clientSecret", "Client/App Secret"),
    ],
    sourceStatus: "official_wire_contract",
  },
  {
    id: "feishu",
    nameZh: "飞书",
    nameEn: "Feishu",
    iconAsset: "integration-icons/feishu.svg",
    transport: "long_connection",
    authMethod: "client_secret",
    capabilities: ["api_access", "messaging", "dm", "group", "topic"],
    credentialFields: [fields.text("appId", "App ID"), fields.secret("appSecret", "App Secret")],
    sourceStatus: "official_wire_contract",
  },
  {
    id: "wecom_ai_bot",
    nameZh: "企微 AI Bot",
    nameEn: "WeCom AI Bot",
    iconAsset: "integration-icons/wecom.ico",
    transport: "web_socket",
    authMethod: "bot_secret",
    capabilities: ["messaging", "dm", "group"],
    credentialFields: [fields.text("botId", "Bot ID"), fields.secret("secret", "Secret")],
    sourceStatus: "official_wire_contract",
  },
  {
    id: "wecom_app",
    nameZh: "企微自建应用",
    nameEn: "WeCom custom app",
    iconAsset: "integration-icons/wecom.ico",
    transport: "encrypted_callback",
    authMethod: "callback_secret",
    capabilities: ["api_access", "messaging", "dm"],
    credentialFields: [
      fields.text("corpId", "Corp ID"),
      fields.secret("corpSecret", "Corp Secret"),
      fields.text("agentId", "Agent ID"),
      fields.secret("callbackToken", "Callback Token"),
      fields.secret("encodingAesKey", "Encoding AES Key"),
      fields.text("externalHttpsUrl", "External HTTPS URL"),
    ],
    sourceStatus: "official_wire_contract",
  },
  {
    id: "wechat_ilink",
    nameZh: "微信 iLink / ClawBot",
    nameEn: "WeChat iLink / ClawBot",
    iconAsset: "integration-icons/wechat-ilink.svg",
    transport: "qr_long_poll",
    authMethod: "qr_code",
    capabilities: ["messaging", "dm", "qr_login"],
    credentialFields: [],
    sourceStatus: "public_qualification_unverified",
  },
];

const gateway: GatewayHealth = {
  running: false,
  state: "starting",
  lastHeartbeatMs: null,
  lastStartedAtMs: null,
  restartAttempt: 0,
  lastErrorCode: null,
  channels: [],
  pendingIngress: 0,
  pendingDeliveries: 0,
  revision: 1,
};
const account: IntegrationProviderAccount = {
  id: "wecom-app-1",
  displayName: "客服账户",
  providerId: "wecom_app",
  connectorAccountId: "integration:wecom-app-1",
  channelAccountId: "wecom-app-1",
  tenantIdentityHash: "tenant-hash",
  transport: "encrypted_callback",
  state: "healthy",
  diagnostic: null,
  apiAccessEnabled: true,
  messagingEnabled: true,
  authorizations: [],
  credentialRevision: 2,
  configRevision: 4,
  updatedAtMs: 10,
  lastEventAtMs: null,
  lastDeliveryAtMs: null,
  lastHandshakeAtMs: null,
  lastFrameAtMs: null,
  lastErrorCode: null,
  nextReconnectAtMs: null,
  consecutiveFailures: 0,
  probe: null,
};

function mount() {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(
    () => (
      <I18nProvider initialLocale="zh-CN">
        <PlatformIntegrationsSettings />
      </I18nProvider>
    ),
    host,
  );
  return { host, dispose };
}

function button(host: HTMLElement, label: string) {
  return [...host.querySelectorAll<HTMLButtonElement>("button")].find(
    (candidate) => candidate.textContent?.trim() === label,
  );
}

function input(host: HTMLElement, label: string, value: string) {
  const field = host.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;
  field.value = value;
  field.dispatchEvent(new InputEvent("input", { bubbles: true }));
}

function select(host: HTMLElement, label: string, value: string) {
  const field = host.querySelector<HTMLSelectElement>(`select[aria-label="${label}"]`)!;
  field.value = value;
  field.dispatchEvent(new Event("change", { bubbles: true }));
}

beforeEach(() => {
  commandMocks.listIntegrationProviders.mockResolvedValue(providers);
  commandMocks.listEnterpriseIntegrations.mockResolvedValue([]);
  commandMocks.getGatewayHealth.mockResolvedValue(gateway);
  commandMocks.upsertEnterpriseIntegration.mockResolvedValue(account);
  commandMocks.setEnterpriseIntegrationCapabilities.mockResolvedValue(account);
  commandMocks.probeEnterpriseIntegration.mockResolvedValue({
    account,
    credential: { ok: true, resultCode: "ok", diagnostic: null },
    ingress: { ok: true, resultCode: "ok", diagnostic: null },
    egress: { ok: true, resultCode: "ok", diagnostic: null },
    api: { ok: true, resultCode: "ok", diagnostic: null },
  });
  commandMocks.removeEnterpriseIntegration.mockResolvedValue(true);
  commandMocks.listChannelAuthorizations.mockResolvedValue([]);
  commandMocks.upsertChannelAuthorization.mockImplementation(async (value) => ({
    ...value,
    revision: (value.expectedRevision ?? 0) + 1,
    createdAtMs: 10,
    updatedAtMs: 10,
  }));
  commandMocks.listChannelIdentityTransferPreviews.mockResolvedValue([]);
  commandMocks.transferChannelIdentity.mockResolvedValue({
    identityGroup: { id: "group-new", revision: 1, createdAtMs: 10, updatedAtMs: 10 },
    previousSourceGroupId: "group-source",
    previousTargetGroupId: "group-target",
    sessionId: "session-new",
  });
  const policy = {
    accountId: account.id,
    dmPolicy: "pairing",
    allowlistActorIds: [],
    grantCeiling: {
      permissionPolicy: {
        level: "read_only",
        rules: {
          fileSystem: [],
          network: { enabled: false, hosts: [], protocols: [] },
          process: { spawn: false, interactive: false, allowedCommands: [] },
          browser: {
            observe: false,
            act: false,
            upload: false,
            download: false,
            cookieStorage: false,
            cdp: false,
            origins: [],
          },
          computer: { observe: false, act: false, allowedApplications: [], maxActions: null },
          mcp: [],
          connectors: [],
        },
        revision: 0,
      },
      skillIds: [],
      mcpServerIds: [],
      connectorSelections: [],
      readOnlyWorkspaceRoots: [],
      networkHosts: [],
    },
    revision: 1,
  } as const;
  commandMocks.getChannelAccessPolicy.mockResolvedValue(policy);
  commandMocks.updateChannelAccessPolicy.mockImplementation(async (input) => ({
    ...policy,
    ...input,
    revision: 2,
  }));
  commandMocks.createChannelPairingCode.mockResolvedValue({
    id: "code-1",
    code: "0123456789ABCDEFGHJKMNPQRS",
    accountId: account.id,
    target: "group_conversation",
    expiresAtMs: Date.now() + 600_000,
  });
  commandMocks.createChannelIdentityLinkCode.mockResolvedValue({
    id: "link-1",
    code: "0123456789ABCDEFGHJKMNPQRS",
    accountId: account.id,
    actorId: "actor-1",
    expiresAtMs: Date.now() + 600_000,
  });
  const qr = {
    accountId: "ilink-1",
    qrContent: "data:image/png;base64,AAAA",
    state: "waiting",
    expiresAtMs: Date.now() + 120_000,
  };
  commandMocks.beginIlinkQrLogin.mockResolvedValue(qr);
  commandMocks.pollIlinkQrLogin.mockResolvedValue(qr);
  commandMocks.cancelIlinkQrLogin.mockResolvedValue(true);
});

afterEach(() => {
  document.body.replaceChildren();
  vi.clearAllMocks();
});

describe("PlatformIntegrationsSettings", () => {
  it("renders all five formal Providers", async () => {
    const mounted = mount();
    await vi.waitFor(() => expect(mounted.host.textContent).toContain("尚未连接账户"));
    expect(
      [...mounted.host.querySelectorAll('[role="tab"]')].map((tab) =>
        tab.getAttribute("aria-label"),
      ),
    ).toEqual(["钉钉", "飞书", "企微 AI Bot", "企微自建应用", "微信 iLink / ClawBot"]);
    mounted.dispose();
  });

  it("stores a WeCom App account with the new provider credential contract", async () => {
    const mounted = mount();
    await vi.waitFor(() => expect(mounted.host.textContent).toContain("尚未连接账户"));
    mounted.host
      .querySelector<HTMLButtonElement>('[role="tab"][aria-label="企微自建应用"]')
      ?.click();
    mounted.host
      .querySelector<HTMLButtonElement>('[data-testid="integration-provider-wecom_app"] button')
      ?.click();
    input(mounted.host, "账户名称", "订单助手");
    input(mounted.host, "Corp ID", "corp-1");
    input(mounted.host, "Corp Secret", "secret-1");
    input(mounted.host, "Agent ID", "100001");
    input(mounted.host, "Callback Token", "callback-1");
    input(mounted.host, "Encoding AES Key", "aes-1");
    input(mounted.host, "External HTTPS URL", "https://bot.example.com");
    button(mounted.host, "连接并检测")?.click();
    await vi.waitFor(() => expect(commandMocks.upsertEnterpriseIntegration).toHaveBeenCalledOnce());
    expect(commandMocks.upsertEnterpriseIntegration).toHaveBeenCalledWith(
      expect.objectContaining({
        credential: expect.objectContaining({
          providerId: "wecom_app",
          corpId: "corp-1",
          agentId: "100001",
        }),
      }),
    );
    await vi.waitFor(() =>
      expect(commandMocks.probeEnterpriseIntegration).toHaveBeenCalledWith(account.id),
    );
    expect(commandMocks.updateChannelAccessPolicy).not.toHaveBeenCalled();
    mounted.dispose();
  });

  it("requires explicit group pairing policies", async () => {
    commandMocks.listEnterpriseIntegrations.mockResolvedValue([
      { ...account, providerId: "feishu", transport: "long_connection" },
    ]);
    const mounted = mount();
    await vi.waitFor(() => expect(mounted.host.textContent).toContain("客服账户"));
    button(mounted.host, "连接码")?.click();
    button(mounted.host, "群会话")?.click();
    button(mounted.host, "生成")?.click();
    expect(commandMocks.createChannelPairingCode).not.toHaveBeenCalled();
    expect(mounted.host.textContent).toContain("请选择群历史、话题历史和 @ 策略");
    select(mounted.host, "群历史", "shared");
    select(mounted.host, "话题历史", "inherit_group");
    select(mounted.host, "@ 策略", "required");
    button(mounted.host, "生成")?.click();
    await vi.waitFor(() => expect(commandMocks.createChannelPairingCode).toHaveBeenCalledOnce());
    expect(commandMocks.createChannelPairingCode).toHaveBeenCalledWith(
      expect.objectContaining({
        target: "group_conversation",
        groupHistoryPolicy: "shared",
        topicPolicy: "inherit_group",
        mentionPolicy: "required",
      }),
    );
    mounted.dispose();
  });

  it("updates DM policy and the grantable Skill ceiling with revision fencing", async () => {
    commandMocks.listEnterpriseIntegrations.mockResolvedValue([account]);
    const mounted = mount();
    await vi.waitFor(() => expect(mounted.host.textContent).toContain("客服账户"));
    mounted.host
      .querySelector<HTMLButtonElement>('[role="tab"][aria-label="企微自建应用"]')
      ?.click();
    button(mounted.host, "策略与权限")?.click();
    await vi.waitFor(() =>
      expect(commandMocks.getChannelAccessPolicy).toHaveBeenCalledWith(account.id),
    );
    const dmPolicy = await vi.waitFor(() => {
      const field = mounted.host.querySelector<HTMLSelectElement>('select[aria-label="私聊策略"]');
      expect(field).toBeTruthy();
      return field!;
    });
    dmPolicy.value = "open";
    dmPolicy.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.waitFor(() =>
      expect(mounted.host.textContent).toContain("任何私聊发送者都可创建会话。"),
    );
    input(mounted.host, "Skill ID（逗号分隔）", "skill-a, skill-a skill-b");
    button(mounted.host, "保存")?.click();
    await vi.waitFor(() => expect(commandMocks.updateChannelAccessPolicy).toHaveBeenCalledOnce());
    expect(commandMocks.updateChannelAccessPolicy).toHaveBeenCalledWith(
      expect.objectContaining({
        accountId: account.id,
        dmPolicy: "open",
        expectedRevision: 1,
        grantCeiling: expect.objectContaining({ skillIds: ["skill-a", "skill-b"] }),
      }),
    );
    mounted.dispose();
  });

  it("hides and clears scoped Channel fields for full access", async () => {
    commandMocks.listEnterpriseIntegrations.mockResolvedValue([account]);
    const mounted = mount();
    await vi.waitFor(() => expect(mounted.host.textContent).toContain("客服账户"));
    mounted.host
      .querySelector<HTMLButtonElement>('[role="tab"][aria-label="企微自建应用"]')
      ?.click();
    button(mounted.host, "策略与权限")?.click();
    await vi.waitFor(() =>
      expect(commandMocks.getChannelAccessPolicy).toHaveBeenCalledWith(account.id),
    );
    select(mounted.host, "权限档位", "full_access");
    expect(mounted.host.textContent).not.toContain("MCP Servers");
    expect(mounted.host.textContent).not.toContain("网络范围");
    button(mounted.host, "保存")?.click();
    await vi.waitFor(() => expect(commandMocks.updateChannelAccessPolicy).toHaveBeenCalledOnce());
    expect(commandMocks.updateChannelAccessPolicy).toHaveBeenCalledWith(
      expect.objectContaining({
        grantCeiling: expect.objectContaining({
          permissionPolicy: expect.objectContaining({ level: "full_access" }),
          mcpServerIds: [],
          connectorSelections: [],
          readOnlyWorkspaceRoots: [],
          networkHosts: [],
        }),
      }),
    );
    mounted.dispose();
  });

  it("starts iLink with a backend-owned QR login instead of browser credentials", async () => {
    const mounted = mount();
    await vi.waitFor(() => expect(mounted.host.textContent).toContain("尚未连接账户"));
    mounted.host
      .querySelector<HTMLButtonElement>('[role="tab"][aria-label="微信 iLink / ClawBot"]')
      ?.click();
    mounted.host
      .querySelector<HTMLButtonElement>('[data-testid="integration-provider-wechat_ilink"] button')
      ?.click();
    input(mounted.host, "账户名称", "微信助手");
    button(mounted.host, "生成二维码")?.click();
    await vi.waitFor(() => expect(commandMocks.beginIlinkQrLogin).toHaveBeenCalledOnce());
    expect(commandMocks.beginIlinkQrLogin).toHaveBeenCalledWith(
      expect.objectContaining({ displayName: "微信助手" }),
    );
    expect(
      mounted.host.querySelector<HTMLImageElement>('img[alt="微信 iLink 二维码"]')?.src,
    ).toContain("data:image/png;base64,AAAA");
    expect(commandMocks.upsertEnterpriseIntegration).not.toHaveBeenCalled();
    mounted.dispose();
  });

  it("creates a manual DM authorization with a revision-fenced grant", async () => {
    commandMocks.listEnterpriseIntegrations.mockResolvedValue([account]);
    const mounted = mount();
    await vi.waitFor(() => expect(mounted.host.textContent).toContain("客服账户"));
    mounted.host
      .querySelector<HTMLButtonElement>('[role="tab"][aria-label="企微自建应用"]')
      ?.click();
    button(mounted.host, "0")?.click();
    await vi.waitFor(() =>
      expect(commandMocks.listChannelAuthorizations).toHaveBeenCalledWith(account.id),
    );
    button(mounted.host, "添加")?.click();
    input(mounted.host, "私聊会话 ID", "dm-user-1");
    input(mounted.host, "发送者 ID", "user-1");
    input(mounted.host, "Network hosts", "api.example.com");
    button(mounted.host, "保存")?.click();
    await vi.waitFor(() => expect(commandMocks.upsertChannelAuthorization).toHaveBeenCalledOnce());
    expect(commandMocks.upsertChannelAuthorization).toHaveBeenCalledWith(
      expect.objectContaining({
        accountId: account.id,
        target: "dm_identity",
        actorId: "user-1",
        expectedRevision: null,
        grant: expect.objectContaining({ networkHosts: ["api.example.com"] }),
      }),
    );
    mounted.dispose();
  });

  it("submits identity transfer with all preview revisions", async () => {
    const preview = {
      id: "transfer-1",
      source: {
        externalIdentityId: "source-identity",
        providerId: "wecom_app",
        accountId: account.id,
        tenantKey: "tenant",
        actorId: "source-user",
        displayName: "来源用户",
        identityGroupId: "group-source",
      },
      target: {
        externalIdentityId: "target-identity",
        providerId: "feishu",
        accountId: "feishu-1",
        tenantKey: "tenant",
        actorId: "target-user",
        displayName: "目标用户",
        identityGroupId: "group-target",
      },
      sourceGroupId: "group-source",
      targetGroupId: "group-target",
      sourceGroupRevision: 4,
      targetGroupRevision: 7,
      revision: 3,
      expiresAtMs: Date.now() + 60_000,
    } as const;
    commandMocks.listEnterpriseIntegrations.mockResolvedValue([account]);
    commandMocks.listChannelIdentityTransferPreviews.mockResolvedValue([preview]);
    const mounted = mount();
    await vi.waitFor(() => expect(mounted.host.textContent).toContain("客服账户"));
    mounted.host
      .querySelector<HTMLButtonElement>('[role="tab"][aria-label="企微自建应用"]')
      ?.click();
    button(mounted.host, "身份")?.click();
    await vi.waitFor(() => expect(mounted.host.textContent).toContain("来源用户"));
    button(mounted.host, "确认转移")?.click();
    await vi.waitFor(() => expect(commandMocks.transferChannelIdentity).toHaveBeenCalledOnce());
    expect(commandMocks.transferChannelIdentity).toHaveBeenCalledWith({
      id: preview.id,
      expectedRevision: preview.revision,
      expectedSourceGroupRevision: preview.sourceGroupRevision,
      expectedTargetGroupRevision: preview.targetGroupRevision,
    });
    mounted.dispose();
  });

  it("shows persisted four-dimensional probe diagnostics and capability-specific controls", async () => {
    commandMocks.listEnterpriseIntegrations.mockResolvedValue([
      {
        ...account,
        lastEventAtMs: 100,
        lastDeliveryAtMs: 200,
        lastHandshakeAtMs: 300,
        lastFrameAtMs: 400,
        lastErrorCode: "provider_transport_unavailable",
        nextReconnectAtMs: 500,
        consecutiveFailures: 2,
        probe: {
          credential: { ok: true, resultCode: "credential_ok", diagnostic: null },
          ingress: { ok: false, resultCode: "callback_unreachable", diagnostic: "HTTP 404" },
          egress: { ok: true, resultCode: "egress_ok", diagnostic: null },
          api: { ok: true, resultCode: "api_ok", diagnostic: null },
          probedAtMs: 300,
        },
      },
    ]);
    const mounted = mount();
    await vi.waitFor(() => expect(mounted.host.textContent).toContain("客服账户"));
    mounted.host
      .querySelector<HTMLButtonElement>('[role="tab"][aria-label="企微自建应用"]')
      ?.click();
    await vi.waitFor(() => expect(mounted.host.textContent).toContain("credential"));
    expect(mounted.host.textContent).toContain("消息连接暂时中断");
    expect(mounted.host.textContent).toContain("provider_transport_unavailable");
    expect(mounted.host.textContent).toContain("ingress");
    expect(mounted.host.textContent).toContain("egress");
    expect(mounted.host.textContent).toContain("api");
    expect(mounted.host.textContent).toContain(`/v1/channels/wecom_app/${account.id}/callback`);

    mounted.host
      .querySelector<HTMLButtonElement>('[role="tab"][aria-label="微信 iLink / ClawBot"]')
      ?.click();
    mounted.host
      .querySelector<HTMLButtonElement>('[data-testid="integration-provider-wechat_ilink"] button')
      ?.click();
    expect(mounted.host.querySelector('[role="switch"][aria-label="企业 API"]')).toBeNull();
    mounted.dispose();
  });
});
