import {
  PLUGIN_UI_BRIDGE_PROTOCOL_VERSION,
  commandFailure,
  commands,
  type BrowserPairing,
  type BrowserCapability,
  type BrowserHostSettings,
  type BrowserPermissionLedgerEntry,
  type BrowserPermissionRequest,
  type BrowserPermissionDecision,
  type EmbeddedBrowserPermissionRequest,
  type EmbeddedBrowserSitePermission,
  type ChannelProviderHealth,
  type ChannelProviderAccount,
  type ChannelProviderManifest,
  type ComputerAppRule,
  type ComputerWindowIdentity,
  type ConnectorAccount,
  type FeatureFlags,
  type GatewayHealth,
  type InstalledContribution,
  type InstalledPlugin,
  type PluginContributionSurface,
  type PluginLifecycleJournalRecord,
  type PluginPermissionDiff,
  type PluginRevisionRecord,
  type PluginUiBridgeRequest,
  type PluginUiBridgeResponse,
  type SandboxBootstrapState,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  Badge,
  Button,
  PageHeading,
  RefreshCw,
  SettingsCard,
  SettingsRow,
  SettingsSection,
  ShieldCheck,
  Select,
  StatusBanner,
  Switch as Toggle,
  TextField,
} from "@hachimi/ui";
import { For, Show, createMemo, createSignal, onCleanup, onMount, untrack } from "solid-js";

import { BrowserSettingsSection } from "./browser-settings";
import { runtimeFeatureVisibility } from "./runtime-feature-visibility";

type BusyAction = "load" | "sandbox" | "plugin" | "gateway" | "pairing" | "channel" | "computer";
type EnterprisePlatformId = "wecom" | "ding_talk" | "feishu";

function isEnterpriseRuntimeId(value: string) {
  return value === "wecom" || value === "dingtalk" || value === "ding_talk" || value === "feishu";
}

const BROWSER_CAPABILITIES: BrowserCapability[] = [
  "observe",
  "act",
  "upload",
  "download",
  "cookie_storage",
  "cdp",
];

export function LocalHostsSettingsPage(props: { featureFlags: FeatureFlags }) {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const featureVisibility = untrack(() => runtimeFeatureVisibility(props.featureFlags));
  const pluginRuntimeEnabled = () => featureVisibility.pluginRuntime;
  const enterpriseEnabled = () => featureVisibility.enterpriseIntegrations;
  const [sandbox, setSandbox] = createSignal<SandboxBootstrapState>();
  const [plugins, setPlugins] = createSignal<InstalledPlugin[]>([]);
  const [pluginContributions, setPluginContributions] = createSignal<InstalledContribution[]>([]);
  const [pluginPermissionDiffs, setPluginPermissionDiffs] = createSignal<
    Record<string, PluginPermissionDiff>
  >({});
  const [pluginRevisions, setPluginRevisions] = createSignal<
    Record<string, PluginRevisionRecord[]>
  >({});
  const [pluginLifecycle, setPluginLifecycle] = createSignal<
    Record<string, PluginLifecycleJournalRecord[]>
  >({});
  const [pluginSurfaces, setPluginSurfaces] = createSignal<
    Record<string, PluginContributionSurface>
  >({});
  const [openPluginSurface, setOpenPluginSurface] = createSignal<PluginContributionSurface>();
  let pluginFrame: HTMLIFrameElement | undefined;
  const [connectorAccounts, setConnectorAccounts] = createSignal<ConnectorAccount[]>([]);
  const [connectorName, setConnectorName] = createSignal("Local sample CRM");
  const [connectorSecret, setConnectorSecret] = createSignal("");
  const [enterprisePlatform, setEnterprisePlatform] = createSignal<EnterprisePlatformId>("wecom");
  const [enterpriseConnectorName, setEnterpriseConnectorName] = createSignal("企业微信账户");
  const [enterpriseCredential, setEnterpriseCredential] = createSignal("");
  const [gateway, setGateway] = createSignal<GatewayHealth>();
  const [channelProviders, setChannelProviders] = createSignal<ChannelProviderHealth[]>([]);
  const [channelManifests, setChannelManifests] = createSignal<ChannelProviderManifest[]>([]);
  const [channelAccounts, setChannelAccounts] = createSignal<ChannelProviderAccount[]>([]);
  const [channelProviderId, setChannelProviderId] = createSignal("");
  const [channelAccountId, setChannelAccountId] = createSignal("local");
  const [channelDisplayName, setChannelDisplayName] = createSignal("Local channel account");
  const [channelCredential, setChannelCredential] = createSignal("");
  const [channelPeer, setChannelPeer] = createSignal("local-user");
  const [channelThread, setChannelThread] = createSignal("main");
  const [channelExpectedRevision, setChannelExpectedRevision] = createSignal<number>();
  const [pairing, setPairing] = createSignal<BrowserPairing>();
  const [browserSettings, setBrowserSettings] = createSignal<BrowserHostSettings>();
  const [browserPermissions, setBrowserPermissions] = createSignal<BrowserPermissionLedgerEntry[]>(
    [],
  );
  const [browserPermissionRequests, setBrowserPermissionRequests] = createSignal<
    BrowserPermissionRequest[]
  >([]);
  const [embeddedPermissionRequests, setEmbeddedPermissionRequests] = createSignal<
    EmbeddedBrowserPermissionRequest[]
  >([]);
  const [embeddedSitePermissions, setEmbeddedSitePermissions] = createSignal<
    EmbeddedBrowserSitePermission[]
  >([]);
  const [computerWindows, setComputerWindows] = createSignal<ComputerWindowIdentity[]>([]);
  const [computerRules, setComputerRules] = createSignal<ComputerAppRule[]>([]);
  const [busy, setBusy] = createSignal<BusyAction>();
  const [failure, setFailure] = createSignal<string>();
  const [notice, setNotice] = createSignal<string>();

  const sandboxReady = createMemo(() => {
    const report = sandbox()?.snapshot.report;
    return (
      sandbox()?.phase === "ready" &&
      report?.readiness === "ready" &&
      report.osEnforced &&
      report.filesystemEnforced &&
      report.processEnforced &&
      report.networkEnforced
    );
  });
  const computerApps = createMemo(() => {
    const seen = new Set<string>();
    return computerWindows().filter((target) => {
      if (seen.has(target.appId)) return false;
      seen.add(target.appId);
      return true;
    });
  });
  const visiblePlugins = (values: InstalledPlugin[]) =>
    enterpriseEnabled()
      ? values
      : values.filter((plugin) => !isEnterpriseRuntimeId(plugin.manifest.id));
  const visibleContributions = (values: InstalledContribution[]) =>
    enterpriseEnabled() ? values : values.filter((entry) => !isEnterpriseRuntimeId(entry.pluginId));
  const visibleConnectorAccounts = (values: ConnectorAccount[]) =>
    enterpriseEnabled()
      ? values
      : values.filter((account) => !isEnterpriseRuntimeId(account.pluginId));
  const visibleChannelAccounts = (values: ChannelProviderAccount[]) =>
    enterpriseEnabled()
      ? values
      : values.filter((account) => !isEnterpriseRuntimeId(account.providerId));
  const visibleChannelHealth = (values: ChannelProviderHealth[]) =>
    enterpriseEnabled()
      ? values
      : values.filter((provider) => !isEnterpriseRuntimeId(provider.providerId));

  async function run(action: BusyAction, operation: () => Promise<void>) {
    setBusy(action);
    setFailure(undefined);
    setNotice(undefined);
    try {
      await operation();
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(undefined);
    }
  }

  async function load() {
    // eslint-disable-next-line solid/reactivity -- load is invoked from tracked mount and explicit UI refresh handlers.
    await run("load", async () => {
      const tasks: Promise<void>[] = [
        commands.getSandboxBootstrapState().then((value) => {
          setSandbox(value);
        }),
      ];
      if (pluginRuntimeEnabled()) {
        tasks.push(
          commands.localHostCommand({ kind: "plugin_list" }).then((response) => {
            if (response.kind === "plugins") {
              setPlugins(
                enterpriseEnabled()
                  ? response.value
                  : response.value.filter((plugin) => !isEnterpriseRuntimeId(plugin.manifest.id)),
              );
              void refreshPluginPermissionDiffs(response.value);
              void refreshPluginLifecycle(response.value);
            }
          }),
          commands
            .localHostCommand({ kind: "plugin_list_contributions", plugin_id: null })
            .then(async (response) => {
              if (response.kind === "plugin_contributions") {
                const contributions = enterpriseEnabled()
                  ? response.value
                  : response.value.filter((entry) => !isEnterpriseRuntimeId(entry.pluginId));
                setPluginContributions(contributions);
                await refreshPluginSurfaces(contributions);
              }
            }),
          commands.localHostCommand({ kind: "connector_list_accounts" }).then((response) => {
            if (response.kind === "connector_accounts") {
              setConnectorAccounts(
                enterpriseEnabled()
                  ? response.value
                  : response.value.filter((account) => !isEnterpriseRuntimeId(account.pluginId)),
              );
            }
          }),
        );
      }
      if (props.featureFlags.localGateway) {
        tasks.push(
          commands.localHostCommand({ kind: "gateway_provider_manifests" }).then((response) => {
            if (response.kind === "channel_provider_manifests") {
              const manifests = enterpriseEnabled()
                ? response.value
                : response.value.filter((manifest) => !isEnterpriseRuntimeId(manifest.id));
              setChannelManifests(manifests);
              setChannelProviderId((current) => current || manifests[0]?.id || "");
            }
          }),
          commands.localHostCommand({ kind: "gateway_health" }).then((response) => {
            if (response.kind === "gateway_health") setGateway(response.value);
          }),
          commands.localHostCommand({ kind: "gateway_provider_health" }).then((response) => {
            if (response.kind === "channel_provider_health") {
              setChannelProviders(
                enterpriseEnabled()
                  ? response.value
                  : response.value.filter(
                      (provider) => !isEnterpriseRuntimeId(provider.providerId),
                    ),
              );
            }
          }),
          commands.localHostCommand({ kind: "gateway_list_provider_accounts" }).then((response) => {
            if (response.kind === "channel_provider_accounts") {
              setChannelAccounts(
                enterpriseEnabled()
                  ? response.value
                  : response.value.filter((account) => !isEnterpriseRuntimeId(account.providerId)),
              );
            }
          }),
        );
      }
      if (props.featureFlags.computerObserve) {
        tasks.push(
          commands.localHostCommand({ kind: "computer_list_windows" }).then((response) => {
            if (response.kind === "computer_windows") setComputerWindows(response.value);
          }),
          commands.localHostCommand({ kind: "computer_list_global_app_rules" }).then((response) => {
            if (response.kind === "computer_rules") setComputerRules(response.value);
          }),
        );
      }
      if (props.featureFlags.browserControl) {
        tasks.push(
          commands.localHostCommand({ kind: "browser_get_host_settings" }).then((response) => {
            if (response.kind === "browser_host_settings") {
              setBrowserSettings(response.value);
              if (response.value.latestPairing) setPairing(response.value.latestPairing);
            }
          }),
          commands.localHostCommand({ kind: "browser_list_permissions" }).then((response) => {
            if (response.kind === "browser_permissions") setBrowserPermissions(response.value);
          }),
          commands
            .localHostCommand({ kind: "browser_list_permission_requests" })
            .then((response) => {
              if (response.kind === "browser_permission_requests") {
                setBrowserPermissionRequests(response.value);
              }
            }),
          commands
            .listEmbeddedBrowserPermissionRequests(null)
            .then((value) => void setEmbeddedPermissionRequests(value)),
          commands
            .listEmbeddedBrowserSitePermissions()
            .then((value) => void setEmbeddedSitePermissions(value)),
        );
      }
      await Promise.all(tasks);
    });
  }

  async function refreshPluginPermissionDiffs(installed = plugins()) {
    const entries = await Promise.all(
      installed.map(async (plugin) => {
        const response = await commands.localHostCommand({
          kind: "plugin_permission_diff",
          plugin_id: plugin.manifest.id,
        });
        return response.kind === "plugin_permission_diff" && response.value
          ? ([plugin.manifest.id, response.value] as const)
          : undefined;
      }),
    );
    setPluginPermissionDiffs(
      Object.fromEntries(entries.filter(Boolean) as [string, PluginPermissionDiff][]),
    );
  }

  async function refreshPluginLifecycle(installed = plugins()) {
    const entries = await Promise.all(
      installed.map(async (plugin) => {
        const [revisions, journal] = await Promise.all([
          commands.localHostCommand({
            kind: "plugin_list_revisions",
            plugin_id: plugin.manifest.id,
          }),
          commands.localHostCommand({
            kind: "plugin_lifecycle_journal",
            plugin_id: plugin.manifest.id,
          }),
        ]);
        return [
          plugin.manifest.id,
          revisions.kind === "plugin_revisions" ? revisions.value : [],
          journal.kind === "plugin_lifecycle_journal" ? journal.value : [],
        ] as const;
      }),
    );
    setPluginRevisions(Object.fromEntries(entries.map(([id, revisions]) => [id, revisions])));
    setPluginLifecycle(Object.fromEntries(entries.map(([id, , journal]) => [id, journal])));
  }

  async function refreshPluginSurfaces(contributions = pluginContributions()) {
    const visible = contributions.filter((entry) =>
      ["hook", "asset", "custom_ui"].includes(entry.kind),
    );
    const surfaces = await Promise.all(
      visible.map(async (entry) => {
        const response = await commands.localHostCommand({
          kind: "plugin_get_contribution_surface",
          plugin_id: entry.pluginId,
          contribution_id: entry.contributionId,
        });
        return response.kind === "plugin_contribution_surface"
          ? ([`${entry.pluginId}:${entry.contributionId}`, response.value] as const)
          : undefined;
      }),
    );
    setPluginSurfaces(
      Object.fromEntries(surfaces.filter(Boolean) as [string, PluginContributionSurface][]),
    );
  }

  async function repairSandbox() {
    await run("sandbox", async () => {
      const bootstrap = await commands.getBootstrapState();
      await commands.repairSandbox({
        context: {
          requestId: crypto.randomUUID(),
          clientId: "window:workbench",
          protocolVersion: bootstrap.protocolVersion,
          idempotencyKey: crypto.randomUUID(),
          expectedRunId: null,
          expectedGeneration: null,
        },
      });
      setSandbox(await commands.getSandboxBootstrapState());
      setNotice(zh() ? "沙箱修复和重新证明已完成。" : "Sandbox repair and attestation completed.");
    });
  }

  async function installPlugin(bundle: boolean) {
    const useChinese = zh();
    // eslint-disable-next-line solid/reactivity -- invoked only from an explicit UI event; nested refreshes intentionally update signals.
    await run("plugin", async () => {
      const response = await commands.localHostCommand({
        kind: bundle ? "plugin_choose_and_install_bundle" : "plugin_choose_and_install",
      });
      if (response.kind === "cancelled") return;
      if (response.kind !== "plugin" || !response.value) {
        throw new Error("plugin_install_protocol_mismatch");
      }
      const next = await commands.localHostCommand({ kind: "plugin_list" });
      if (next.kind === "plugins") {
        setPlugins(visiblePlugins(next.value));
        await refreshPluginPermissionDiffs(next.value);
        await refreshPluginLifecycle(next.value);
      }
      const contributions = await commands.localHostCommand({
        kind: "plugin_list_contributions",
        plugin_id: null,
      });
      if (contributions.kind === "plugin_contributions") {
        setPluginContributions(visibleContributions(contributions.value));
        await refreshPluginSurfaces(contributions.value);
      }
      setNotice(
        useChinese
          ? `已导入 ${response.value.manifest.name}；启用前请检查贡献点和权限。`
          : `${response.value.manifest.name} imported; review contributions and permissions before enabling.`,
      );
    });
  }

  async function installBuiltinSampleCrm() {
    const useChinese = zh();
    // eslint-disable-next-line solid/reactivity -- invoked only from an explicit UI event; nested refreshes intentionally update signals.
    await run("plugin", async () => {
      const response = await commands.localHostCommand({
        kind: "plugin_install_builtin_sample_crm",
      });
      if (response.kind !== "plugin" || !response.value) {
        throw new Error("builtin_sample_crm_protocol_mismatch");
      }
      const next = await commands.localHostCommand({ kind: "plugin_list" });
      if (next.kind === "plugins") {
        setPlugins(visiblePlugins(next.value));
        await refreshPluginPermissionDiffs(next.value);
        await refreshPluginLifecycle(next.value);
      }
      const contributions = await commands.localHostCommand({
        kind: "plugin_list_contributions",
        plugin_id: null,
      });
      if (contributions.kind === "plugin_contributions") {
        setPluginContributions(visibleContributions(contributions.value));
        await refreshPluginSurfaces(contributions.value);
      }
      setNotice(
        useChinese
          ? "内置 sample-crm 已安装；请显式启用后再创建账户。"
          : "Built-in sample-crm installed; enable it explicitly before creating an account.",
      );
    });
  }

  async function installBuiltinEnterprise(platform: EnterprisePlatformId) {
    const useChinese = zh();
    // eslint-disable-next-line solid/reactivity -- invoked only from an explicit UI event.
    await run("plugin", async () => {
      const response = await commands.localHostCommand({
        kind: "plugin_install_builtin_enterprise",
        platform,
      });
      if (response.kind !== "plugin" || !response.value) {
        throw new Error("builtin_enterprise_protocol_mismatch");
      }
      const next = await commands.localHostCommand({ kind: "plugin_list" });
      if (next.kind === "plugins") {
        setPlugins(visiblePlugins(next.value));
        await refreshPluginPermissionDiffs(next.value);
        await refreshPluginLifecycle(next.value);
      }
      const contributions = await commands.localHostCommand({
        kind: "plugin_list_contributions",
        plugin_id: null,
      });
      if (contributions.kind === "plugin_contributions") {
        setPluginContributions(visibleContributions(contributions.value));
        await refreshPluginSurfaces(contributions.value);
      }
      setNotice(
        useChinese
          ? `${response.value.manifest.name} 已安装；请检查权限并显式启用。`
          : `${response.value.manifest.name} installed; review permissions and enable it explicitly.`,
      );
    });
  }

  async function mutatePlugin(
    plugin: InstalledPlugin,
    operation: "toggle" | "health" | "rollback" | "remove",
  ) {
    const pluginId = plugin.manifest.id;
    const enablePlugin = plugin.status !== "enabled";
    // eslint-disable-next-line solid/reactivity -- all reactive inputs are snapshotted by the calling UI event.
    await run("plugin", async () => {
      if (operation === "remove") {
        await commands.localHostCommand({
          kind: "plugin_uninstall",
          plugin_id: pluginId,
        });
      } else if (operation === "rollback") {
        await commands.localHostCommand({
          kind: "plugin_rollback",
          plugin_id: pluginId,
          revision: null,
        });
      } else {
        await commands.localHostCommand(
          operation === "health"
            ? { kind: "plugin_health_check", plugin_id: pluginId }
            : {
                kind: "plugin_set_enabled",
                plugin_id: pluginId,
                enabled: enablePlugin,
              },
        );
      }
      const response = await commands.localHostCommand({ kind: "plugin_list" });
      if (response.kind === "plugins") {
        setPlugins(visiblePlugins(response.value));
        await refreshPluginPermissionDiffs(response.value);
        await refreshPluginLifecycle(response.value);
      }
      const contributions = await commands.localHostCommand({
        kind: "plugin_list_contributions",
        plugin_id: null,
      });
      if (contributions.kind === "plugin_contributions") {
        setPluginContributions(visibleContributions(contributions.value));
        await refreshPluginSurfaces(contributions.value);
      }
    });
  }

  async function openCustomUi(pluginId: string, contributionId: string) {
    await run("plugin", async () => {
      const response = await commands.localHostCommand({
        kind: "plugin_get_contribution_surface",
        plugin_id: pluginId,
        contribution_id: contributionId,
      });
      if (
        response.kind !== "plugin_contribution_surface" ||
        !response.value.entryUrl ||
        response.value.runtimeState !== "active"
      ) {
        throw new Error("plugin_custom_ui_surface_unavailable");
      }
      setOpenPluginSurface(response.value);
    });
  }

  function closeCustomUi() {
    setOpenPluginSurface(undefined);
  }

  function pluginBridgeRequest(value: unknown): PluginUiBridgeRequest | undefined {
    if (!value || typeof value !== "object") return undefined;
    const request = value as Record<string, unknown>;
    if (typeof request.method !== "string" || typeof request.request_id !== "string") {
      return undefined;
    }
    if (request.method === "get_context" || request.method === "close") {
      return request as PluginUiBridgeRequest;
    }
    if (
      request.method === "resolve_asset_url" &&
      typeof request.asset_contribution_id === "string" &&
      typeof request.relative_path === "string"
    ) {
      return request as PluginUiBridgeRequest;
    }
    return undefined;
  }

  function safeAssetPath(value: string) {
    const segments = value.replaceAll("\\", "/").split("/");
    if (
      !segments.length ||
      segments.some((segment) => !segment || segment === "." || segment === "..")
    ) {
      return undefined;
    }
    return segments.map(encodeURIComponent).join("/");
  }

  async function handlePluginBridge(event: MessageEvent) {
    const surface = openPluginSurface();
    if (!surface || !pluginFrame || event.source !== pluginFrame.contentWindow) return;
    const envelope = event.data as Record<string, unknown> | undefined;
    if (
      envelope?.source !== "hachimi-plugin-ui" ||
      envelope.protocolVersion !== PLUGIN_UI_BRIDGE_PROTOCOL_VERSION
    )
      return;
    const request = pluginBridgeRequest(envelope.request);
    if (!request) return;
    let response: PluginUiBridgeResponse;
    if (request.method === "get_context") {
      response = {
        kind: "context",
        request_id: request.request_id,
        value: {
          pluginId: surface.pluginId,
          contributionId: surface.contributionId,
          runtimeRevision: surface.runtimeRevision,
          locale: i18n.locale(),
          theme: document.documentElement.dataset.theme ?? "system",
        },
      };
    } else if (request.method === "close") {
      response = { kind: "closed", request_id: request.request_id };
      closeCustomUi();
    } else {
      const path = safeAssetPath(request.relative_path);
      const installed = pluginContributions().some(
        (entry) =>
          entry.pluginId === surface.pluginId &&
          entry.contributionId === request.asset_contribution_id &&
          entry.kind === "asset" &&
          entry.state === "active",
      );
      const asset = installed
        ? await commands.localHostCommand({
            kind: "plugin_get_contribution_surface",
            plugin_id: surface.pluginId,
            contribution_id: request.asset_contribution_id,
          })
        : undefined;
      response =
        path && asset?.kind === "plugin_contribution_surface" && asset.value.assetBaseUrl
          ? {
              kind: "asset_url",
              request_id: request.request_id,
              value: `${asset.value.assetBaseUrl}${path}`,
            }
          : {
              kind: "error",
              request_id: request.request_id,
              code: "plugin_asset_surface_unavailable",
            };
    }
    pluginFrame.contentWindow?.postMessage(
      {
        source: "hachimi-plugin-host",
        protocolVersion: PLUGIN_UI_BRIDGE_PROTOCOL_VERSION,
        response,
      },
      "*",
    );
  }

  window.addEventListener("message", handlePluginBridge);
  onCleanup(() => window.removeEventListener("message", handlePluginBridge));

  async function refreshConnectorAccounts() {
    const response = await commands.localHostCommand({ kind: "connector_list_accounts" });
    if (response.kind !== "connector_accounts")
      throw new Error("connector_accounts_protocol_mismatch");
    setConnectorAccounts(visibleConnectorAccounts(response.value));
  }

  async function createSampleAccount() {
    const plugin = plugins().find(
      (entry) => entry.manifest.id === "sample-crm" && entry.status === "enabled",
    );
    if (!plugin) {
      setFailure(zh() ? "请先导入并启用 sample-crm 插件。" : "Import and enable sample-crm first.");
      return;
    }
    // eslint-disable-next-line solid/reactivity -- the callback is executed synchronously from this UI event flow.
    await run("plugin", async () => {
      const response = await commands.localHostCommand({
        kind: "connector_upsert_account",
        account: {
          id: crypto.randomUUID(),
          pluginId: plugin.manifest.id,
          connectorId: "sample-crm",
          displayName: connectorName().trim() || "Local sample CRM",
          secret: connectorSecret() || null,
        },
      });
      if (response.kind !== "connector_account" || !response.value) {
        throw new Error("connector_account_protocol_mismatch");
      }
      setConnectorSecret("");
      await refreshConnectorAccounts();
      setNotice(
        zh()
          ? "Connector 账户已创建；revision 由 Host 计算，secret 仅保存到系统凭据库。"
          : "Connector account created; the Host computed its revision and stored the secret only in the OS credential store.",
      );
    });
  }

  async function createEnterpriseAccount() {
    const platform = enterprisePlatform();
    const pluginId = platform === "ding_talk" ? "dingtalk" : platform;
    const plugin = plugins().find(
      (entry) => entry.manifest.id === pluginId && entry.status === "enabled",
    );
    if (!plugin) {
      setFailure(
        zh()
          ? `请先安装并启用 ${pluginId} 插件。`
          : `Install and enable the ${pluginId} plugin first.`,
      );
      return;
    }
    const credential = enterpriseCredential().trim();
    if (!credential) {
      setFailure(zh() ? "企业平台凭据 JSON 不能为空。" : "Enterprise credential JSON is required.");
      return;
    }
    // eslint-disable-next-line solid/reactivity -- inputs are snapshotted before the async mutation.
    await run("plugin", async () => {
      const connectorId = pluginId;
      const response = await commands.localHostCommand({
        kind: "connector_upsert_account",
        account: {
          id: crypto.randomUUID(),
          pluginId,
          connectorId,
          displayName: enterpriseConnectorName().trim() || plugin.manifest.name,
          secret: credential,
        },
      });
      if (response.kind !== "connector_account" || !response.value) {
        throw new Error("enterprise_connector_account_protocol_mismatch");
      }
      setEnterpriseCredential("");
      await refreshConnectorAccounts();
      setNotice(
        zh()
          ? "企业 Connector 账户已创建；凭据正文仅写入系统凭据库。"
          : "Enterprise Connector account created; credential text was written only to the OS credential store.",
      );
    });
  }

  async function revokeConnector(account: ConnectorAccount) {
    await run("plugin", async () => {
      await commands.localHostCommand({
        kind: "connector_revoke_account",
        account_id: account.id,
      });
      await refreshConnectorAccounts();
    });
  }

  async function beginPairing() {
    await run("pairing", async () => {
      const response = await commands.localHostCommand({ kind: "browser_begin_pairing" });
      if (response.kind !== "browser_pairing") throw new Error("browser_pairing_protocol_mismatch");
      setPairing(response.value);
    });
  }

  async function updateBrowserPreference(useExtension: boolean) {
    await run("pairing", async () => {
      const response = await commands.localHostCommand({
        kind: "browser_set_preferred_profile",
        profile_kind: useExtension ? "chrome_extension" : "isolated",
      });
      if (response.kind !== "browser_host_settings") {
        throw new Error("browser_host_settings_protocol_mismatch");
      }
      setBrowserSettings(response.value);
      if (response.value.latestPairing) setPairing(response.value.latestPairing);
    });
  }

  async function browserMutationContext(runId: string, generation: number) {
    const bootstrap = await commands.getBootstrapState();
    return {
      requestId: crypto.randomUUID(),
      clientId: "window:workbench",
      protocolVersion: bootstrap.protocolVersion,
      idempotencyKey: crypto.randomUUID(),
      expectedRunId: runId,
      expectedGeneration: generation,
    };
  }

  async function refreshBrowserPermissions() {
    const [permissions, requests, embeddedRequests, embeddedPermissions] = await Promise.all([
      commands.localHostCommand({ kind: "browser_list_permissions" }),
      commands.localHostCommand({ kind: "browser_list_permission_requests" }),
      commands.listEmbeddedBrowserPermissionRequests(null),
      commands.listEmbeddedBrowserSitePermissions(),
    ]);
    if (permissions.kind === "browser_permissions") setBrowserPermissions(permissions.value);
    if (requests.kind === "browser_permission_requests") {
      setBrowserPermissionRequests(requests.value);
    }
    setEmbeddedPermissionRequests(embeddedRequests);
    setEmbeddedSitePermissions(embeddedPermissions);
  }

  async function resolveEmbeddedPermission(
    request: EmbeddedBrowserPermissionRequest,
    decision: BrowserPermissionDecision,
  ) {
    await run("pairing", async () => {
      await commands.resolveEmbeddedBrowserPermission({ requestId: request.id, decision });
      await refreshBrowserPermissions();
    });
  }

  async function revokeEmbeddedPermission(permission: EmbeddedBrowserSitePermission) {
    await run("pairing", async () => {
      await commands.revokeEmbeddedBrowserSitePermission(permission.id);
      await refreshBrowserPermissions();
    });
  }

  async function resolveBrowserPermission(request: BrowserPermissionRequest, allow: boolean) {
    await run("pairing", async () => {
      await commands.localHostCommand({
        kind: "browser_grant_site_permission",
        context: await browserMutationContext(request.ownerRunId, request.runGeneration),
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
      await refreshBrowserPermissions();
    });
  }

  async function revokeBrowserPermission(entry: BrowserPermissionLedgerEntry) {
    await run("pairing", async () => {
      await commands.localHostCommand({
        kind: "browser_revoke_site_permission",
        context: await browserMutationContext(entry.ownerRunId, entry.runGeneration),
        session_id: entry.ownerSessionId,
        run_id: entry.ownerRunId,
        browser_session_id: entry.browserSessionId,
        expected_revision: entry.browserRevision,
        origin: entry.permission.origin,
      });
      await refreshBrowserPermissions();
    });
  }

  async function updateBrowserCapability(
    entry: BrowserPermissionLedgerEntry,
    capability: BrowserCapability,
    enabled: boolean,
  ) {
    await run("pairing", async () => {
      const capabilities = enabled
        ? [...new Set([...entry.permission.capabilities, capability])]
        : entry.permission.capabilities.filter((value) => value !== capability);
      if (capabilities.length === 0) {
        await commands.localHostCommand({
          kind: "browser_revoke_site_permission",
          context: await browserMutationContext(entry.ownerRunId, entry.runGeneration),
          session_id: entry.ownerSessionId,
          run_id: entry.ownerRunId,
          browser_session_id: entry.browserSessionId,
          expected_revision: entry.browserRevision,
          origin: entry.permission.origin,
        });
      } else {
        const rules = entry.networkRules.length
          ? entry.networkRules
          : [
              {
                origin: entry.permission.origin,
                kind: "document" as const,
                allowPrivateNetwork: false,
                expiresAtMs: entry.permission.expiresAtMs,
              },
            ];
        for (const rule of rules) {
          await commands.localHostCommand({
            kind: "browser_grant_site_permission",
            context: await browserMutationContext(entry.ownerRunId, entry.runGeneration),
            session_id: entry.ownerSessionId,
            run_id: entry.ownerRunId,
            browser_session_id: entry.browserSessionId,
            expected_revision: entry.browserRevision,
            origin: entry.permission.origin,
            capabilities,
            decision: entry.permission.decision,
            network_kind: rule.kind,
            allow_private_network: rule.allowPrivateNetwork,
            expires_at_ms: entry.permission.expiresAtMs,
          });
        }
      }
      await refreshBrowserPermissions();
    });
  }

  async function setComputerAlwaysAllowed(target: ComputerWindowIdentity, enabled: boolean) {
    // eslint-disable-next-line solid/reactivity -- invoked only from the explicit application toggle event.
    await run("computer", async () => {
      if (enabled) {
        await commands.localHostCommand({
          kind: "computer_set_global_app_rule",
          rule: {
            appId: target.appId,
            observe: true,
            act: props.featureFlags.computerControl,
            alwaysAllowed: true,
            grantedBy: "",
            updatedAtMs: 0,
          },
        });
      } else {
        await commands.localHostCommand({
          kind: "computer_remove_global_app_rule",
          app_id: target.appId,
        });
      }
      const response = await commands.localHostCommand({ kind: "computer_list_global_app_rules" });
      if (response.kind === "computer_rules") setComputerRules(response.value);
      setNotice(
        enabled
          ? zh()
            ? `${target.appId} 已加入 Always-allowed Apps。`
            : `${target.appId} added to Always-allowed Apps.`
          : zh()
            ? `${target.appId} 已从 Always-allowed Apps 移除。`
            : `${target.appId} removed from Always-allowed Apps.`,
      );
    });
  }

  async function updateGatewayStartup(enabled: boolean) {
    await run("gateway", async () => {
      const response = await commands.localHostCommand({
        kind: "gateway_set_startup_enabled",
        enabled,
      });
      if (response.kind !== "gateway_health") throw new Error("gateway_health_protocol_mismatch");
      setGateway(response.value);
    });
  }

  async function reconcileGateway() {
    await run("gateway", async () => {
      await commands.localHostCommand({ kind: "gateway_reconcile" });
      const response = await commands.localHostCommand({ kind: "gateway_health" });
      if (response.kind === "gateway_health") setGateway(response.value);
      setNotice(
        zh()
          ? "Gateway ledger reconciliation 已完成。"
          : "Gateway ledger reconciliation completed.",
      );
    });
  }

  async function setChannelAccountEnabled(account: ChannelProviderAccount, enabled: boolean) {
    await run("channel", async () => {
      const response = await commands.localHostCommand({
        kind: "gateway_upsert_provider_account",
        account: {
          id: account.id,
          providerId: account.providerId,
          displayName: account.displayName,
          credential: null,
          enabled,
          routeAllowlist: account.routeAllowlist,
          expectedConfigRevision: account.configRevision,
        },
      });
      if (response.kind !== "channel_provider_account") {
        throw new Error("gateway_provider_account_protocol_mismatch");
      }
      const accounts = await commands.localHostCommand({ kind: "gateway_list_provider_accounts" });
      if (accounts.kind === "channel_provider_accounts") {
        setChannelAccounts(visibleChannelAccounts(accounts.value));
      }
      const health = await commands.localHostCommand({ kind: "gateway_provider_health" });
      if (health.kind === "channel_provider_health") {
        setChannelProviders(visibleChannelHealth(health.value));
      }
    });
  }

  function editChannelAccount(account: ChannelProviderAccount) {
    const route = account.routeAllowlist[0];
    setChannelProviderId(account.providerId);
    setChannelAccountId(account.id);
    setChannelDisplayName(account.displayName);
    setChannelCredential("");
    setChannelPeer(route?.peer ?? "local-user");
    setChannelThread(route?.thread ?? "main");
    setChannelExpectedRevision(account.configRevision);
  }

  async function saveChannelAccount() {
    const providerId = channelProviderId().trim();
    const accountId = channelAccountId().trim();
    const displayName = channelDisplayName().trim();
    const credential = channelCredential();
    const expectedRevision = channelExpectedRevision();
    const peer = channelPeer().trim();
    const thread = channelThread().trim();
    const isZh = zh();
    if (!providerId || !accountId || !displayName || !peer || !thread) {
      setFailure(
        zh()
          ? "Provider、账户、显示名称、peer 和 thread 均不能为空。"
          : "Provider, account, display name, peer, and thread are required.",
      );
      return;
    }
    await run("channel", async () => {
      const response = await commands.localHostCommand({
        kind: "gateway_upsert_provider_account",
        account: {
          id: accountId,
          providerId,
          displayName,
          credential: credential || null,
          enabled: true,
          routeAllowlist: [
            {
              channel: providerId,
              account: accountId,
              peer,
              thread,
            },
          ],
          expectedConfigRevision: expectedRevision ?? null,
        },
      });
      if (response.kind !== "channel_provider_account") {
        throw new Error("gateway_provider_account_protocol_mismatch");
      }
      setChannelCredential("");
      setChannelExpectedRevision(response.value.configRevision);
      const [accounts, health] = await Promise.all([
        commands.localHostCommand({ kind: "gateway_list_provider_accounts" }),
        commands.localHostCommand({ kind: "gateway_provider_health" }),
      ]);
      if (accounts.kind === "channel_provider_accounts") {
        setChannelAccounts(visibleChannelAccounts(accounts.value));
      }
      if (health.kind === "channel_provider_health") {
        setChannelProviders(visibleChannelHealth(health.value));
      }
      setNotice(
        isZh
          ? "Channel 账户和 route allowlist 已保存。"
          : "Channel account and route allowlist saved.",
      );
    });
  }

  async function runMockPollSample() {
    await run("channel", async () => {
      const route = {
        channel: "mock-poll",
        account: "local",
        peer: "local-user",
        thread: "main",
      };
      await commands.localHostCommand({ kind: "channel_mock_poll_set_connected", connected: true });
      await commands.localHostCommand({
        kind: "channel_mock_poll_push",
        envelope: {
          messageId: crypto.randomUUID(),
          route,
          sender: "local-demo",
          text: "Deterministic mock-poll health check",
          metadata: { sample: true },
          authenticated: true,
          botGenerated: false,
          receivedAtMs: Date.now(),
        },
      });
      const response = await commands.localHostCommand({ kind: "channel_mock_poll_drain" });
      const count = response.kind === "ingresses" ? response.value.length : 0;
      setNotice(
        zh()
          ? `mock-poll 已持久接收 ${count} 条消息。`
          : `mock-poll durably accepted ${count} message(s).`,
      );
    });
  }

  onMount(() => void load());

  return (
    <div class="settings-page local-hosts-settings" data-testid="local-hosts-settings-page">
      <PageHeading
        class="settings-page-heading"
        title={zh() ? "本地 Agent Hosts" : "Local Agent Hosts"}
        description={
          zh()
            ? "管理普通用户沙箱、Browser/Computer broker、本地插件、Connector 与 Gateway。所有副作用仍经过统一 Run 权限和审批。"
            : "Manage the per-user sandbox, Browser/Computer brokers, local plugins, Connectors, and Gateway. Side effects still use the unified Run policy and approvals."
        }
        actions={
          <Button
            disabled={Boolean(busy())}
            data-testid="local-hosts-refresh"
            onClick={() => void load()}
          >
            <RefreshCw size={14} />
            {zh() ? "刷新" : "Refresh"}
          </Button>
        }
      />

      <Show when={failure()}>
        {(message) => (
          <div data-testid="local-hosts-failure">
            <StatusBanner tone="danger">{message()}</StatusBanner>
          </div>
        )}
      </Show>
      <Show when={notice()}>
        {(message) => (
          <div data-testid="local-hosts-notice">
            <StatusBanner tone="success">{message()}</StatusBanner>
          </div>
        )}
      </Show>

      <SettingsSection title={zh() ? "普通用户沙箱" : "Per-user sandbox"}>
        <SettingsCard>
          <SettingsRow
            label={zh() ? "安装与证明状态" : "Bootstrap and attestation"}
            description={`${sandbox()?.phase ?? "not_started"} · ${sandbox()?.stableErrorCode ?? "no_error"}`}
          >
            <Badge tone={sandboxReady() ? "success" : "warning"}>
              {sandboxReady() ? (zh() ? "已就绪" : "Ready") : zh() ? "需要处理" : "Needs action"}
            </Badge>
          </SettingsRow>
          <SettingsRow
            label={zh() ? "四项强制能力" : "Four enforcement capabilities"}
            description={
              zh()
                ? "OS、文件系统、进程和网络必须全部有效。"
                : "OS, filesystem, process, and network must all be enforced."
            }
          >
            <span class="local-hosts-metrics" data-testid="local-hosts-sandbox-capabilities">
              {
                [
                  sandbox()?.snapshot.report.osEnforced,
                  sandbox()?.snapshot.report.filesystemEnforced,
                  sandbox()?.snapshot.report.processEnforced,
                  sandbox()?.snapshot.report.networkEnforced,
                ].filter(Boolean).length
              }
              /4
            </span>
          </SettingsRow>
          <SettingsRow
            label={zh() ? "无 UAC 修复" : "Repair without UAC"}
            description={
              sandbox()?.runtimeRoot ??
              (zh() ? "运行目录尚未创建" : "Runtime directory not created")
            }
          >
            <Button disabled={Boolean(busy())} onClick={() => void repairSandbox()}>
              <ShieldCheck size={14} />
              {busy() === "sandbox"
                ? zh()
                  ? "正在修复…"
                  : "Repairing…"
                : zh()
                  ? "安装/修复"
                  : "Install/repair"}
            </Button>
          </SettingsRow>
        </SettingsCard>
      </SettingsSection>

      <BrowserSettingsSection featureFlags={props.featureFlags} />

      <SettingsSection title={zh() ? "Browser 与 Computer" : "Browser and Computer"}>
        <SettingsCard>
          <SettingsRow
            label={zh() ? "Browser Agent Router" : "Browser Agent router"}
            description={
              zh()
                ? "统一路由到内置 CEF 或已配对的外置 Chrome。"
                : "Routes automation to embedded CEF or paired external Chrome."
            }
          >
            <Badge tone={props.featureFlags.browserControl ? "success" : "warning"}>
              {props.featureFlags.browserControl
                ? zh()
                  ? "可用"
                  : "Available"
                : zh()
                  ? "已关闭"
                  : "Off"}
            </Badge>
          </SettingsRow>
          <SettingsRow
            label={zh() ? "Chrome 扩展配对" : "Chrome extension pairing"}
            description={
              pairing()
                ? `${pairing()!.id} · ${pairing()!.confirmed ? "confirmed" : "pending"}`
                : zh()
                  ? "一次性 nonce，过期后必须重新配对。"
                  : "One-time nonce; expired pairings must be renewed."
            }
          >
            <div class="local-hosts-actions">
              <Button
                disabled={Boolean(busy()) || !props.featureFlags.browserControl}
                data-testid="local-hosts-browser-pair"
                onClick={() => void beginPairing()}
              >
                {zh() ? "开始配对" : "Begin pairing"}
              </Button>
            </div>
          </SettingsRow>
          <Show when={pairing() && !pairing()!.confirmed}>
            <SettingsRow
              label={zh() ? "一次性配对码" : "One-time pairing code"}
              description={
                zh()
                  ? "在 Hachimi Chrome 扩展弹窗中输入；只有扩展 localhost transport 能确认身份。"
                  : "Enter this in the Hachimi Chrome extension popup; only the extension localhost transport can confirm identity."
              }
            >
              <code data-testid="local-hosts-browser-pairing-code">{pairing()!.nonce}</code>
            </SettingsRow>
          </Show>
          <SettingsRow
            label={zh() ? "Agent 首选 Browser 模式" : "Preferred Agent Browser mode"}
            description={
              zh()
                ? "未显式指定模式的 browser_start 会使用此设置；配对过期仍会 fail closed。"
                : "browser_start uses this setting when no mode is explicit; expired pairing still fails closed."
            }
          >
            <Toggle
              label={zh() ? "优先使用 Chrome 扩展" : "Prefer Chrome extension"}
              checked={browserSettings()?.preferredProfileKind === "chrome_extension"}
              disabled={Boolean(busy()) || !pairing()?.confirmed}
              onChange={(enabled) => void updateBrowserPreference(enabled)}
            />
          </SettingsRow>
          <For
            each={embeddedPermissionRequests().filter((request) => request.status === "pending")}
          >
            {(request) => (
              <SettingsRow
                label={`${zh() ? "内置浏览器待授权" : "Embedded browser request"}: ${request.origin}`}
                description={
                  request.privateNetwork
                    ? zh()
                      ? "Agent 请求访问本机或私有网络"
                      : "Agent requests localhost or private network access"
                    : `${request.capabilities.join(", ")} · Run ${request.ownerRunId}`
                }
              >
                <div class="local-hosts-actions">
                  <Button
                    size="small"
                    disabled={Boolean(busy())}
                    onClick={() => void resolveEmbeddedPermission(request, "allow_once")}
                  >
                    {zh() ? "允许一次" : "Allow once"}
                  </Button>
                  <Button
                    size="small"
                    disabled={Boolean(busy())}
                    onClick={() => void resolveEmbeddedPermission(request, "allow_session")}
                  >
                    {zh() ? "本会话允许" : "Allow session"}
                  </Button>
                  <Button
                    size="small"
                    disabled={Boolean(busy())}
                    onClick={() => void resolveEmbeddedPermission(request, "allow_persisted")}
                  >
                    {zh() ? "始终允许" : "Always allow"}
                  </Button>
                  <Button
                    size="small"
                    variant="danger"
                    disabled={Boolean(busy())}
                    onClick={() => void resolveEmbeddedPermission(request, "deny")}
                  >
                    {zh() ? "拒绝" : "Deny"}
                  </Button>
                </div>
              </SettingsRow>
            )}
          </For>
          <For each={embeddedSitePermissions()}>
            {(permission) => (
              <SettingsRow
                label={permission.origin}
                description={`${zh() ? "内置浏览器 Agent 权限" : "Embedded Agent permission"} · ${permission.scope}${permission.allowPrivateNetwork ? " · private" : ""}`}
              >
                <Button
                  size="small"
                  variant="danger"
                  disabled={Boolean(busy())}
                  onClick={() => void revokeEmbeddedPermission(permission)}
                >
                  {zh() ? "撤销" : "Revoke"}
                </Button>
              </SettingsRow>
            )}
          </For>
          <For each={browserPermissionRequests().filter((request) => request.status === "pending")}>
            {(request) => (
              <SettingsRow
                label={`${zh() ? "待授权 Origin" : "Pending origin"}: ${request.origin}`}
                description={`${request.networkKind} · ${request.capabilities.join(", ")} · ${zh() ? "到期" : "expires"} ${new Date(request.expiresAtMs).toLocaleString()}`}
              >
                <div class="local-hosts-actions">
                  <Button
                    size="small"
                    disabled={Boolean(busy())}
                    onClick={() => void resolveBrowserPermission(request, true)}
                  >
                    {zh() ? "允许本次 Session" : "Allow for Session"}
                  </Button>
                  <Button
                    size="small"
                    variant="danger"
                    disabled={Boolean(busy())}
                    onClick={() => void resolveBrowserPermission(request, false)}
                  >
                    {zh() ? "拒绝" : "Deny"}
                  </Button>
                </div>
              </SettingsRow>
            )}
          </For>
          <For each={browserPermissions()}>
            {(entry) => (
              <SettingsRow
                label={entry.permission.origin}
                description={`${entry.permission.capabilities.join(", ")} · ${entry.networkRules.map((rule) => `${rule.kind}${rule.allowPrivateNetwork ? ":private" : ""}`).join(", ")} · ${entry.permission.expiresAtMs ? `${zh() ? "到期" : "expires"} ${new Date(entry.permission.expiresAtMs).toLocaleString()}` : zh() ? "Session 有效" : "Session lifetime"}`}
              >
                <div class="local-hosts-actions">
                  <For each={BROWSER_CAPABILITIES}>
                    {(capability) => (
                      <Toggle
                        label={capability}
                        checked={entry.permission.capabilities.includes(capability)}
                        disabled={Boolean(busy())}
                        onChange={(enabled) =>
                          void updateBrowserCapability(entry, capability, enabled)
                        }
                      />
                    )}
                  </For>
                  <Button
                    size="small"
                    variant="danger"
                    disabled={Boolean(busy())}
                    onClick={() => void revokeBrowserPermission(entry)}
                  >
                    {zh() ? "撤销站点权限" : "Revoke site permission"}
                  </Button>
                </div>
              </SettingsRow>
            )}
          </For>
          <SettingsRow
            label={zh() ? "Windows Computer Host" : "Windows Computer Host"}
            description={
              zh()
                ? "仅同完整性级别、已授权应用；用户输入会立即废弃旧 Frame。"
                : "Only authorized apps at the same integrity level; user input invalidates stale frames."
            }
          >
            <Badge
              tone={
                props.featureFlags.computerObserve && props.featureFlags.computerControl
                  ? "success"
                  : "warning"
              }
            >
              {props.featureFlags.computerObserve && props.featureFlags.computerControl
                ? zh()
                  ? "可用"
                  : "Available"
                : zh()
                  ? "已关闭"
                  : "Off"}
            </Badge>
          </SettingsRow>
          <For
            each={computerApps()}
            fallback={
              <SettingsRow
                label={zh() ? "可授权的普通应用" : "Eligible user applications"}
                description={
                  zh()
                    ? "当前未发现可见的普通用户窗口。"
                    : "No eligible visible user window is currently available."
                }
              >
                <span class="local-hosts-empty">{zh() ? "暂无" : "None"}</span>
              </SettingsRow>
            }
          >
            {(target) => {
              const allowed = () => computerRules().some((rule) => rule.appId === target.appId);
              return (
                <SettingsRow
                  label={target.appId}
                  description={`${target.title} · ${target.windowHandle}`}
                >
                  <Toggle
                    label={zh() ? "始终允许 Observe/Act" : "Always allow Observe/Act"}
                    checked={allowed()}
                    disabled={Boolean(busy()) || !props.featureFlags.computerObserve}
                    onChange={(enabled) => void setComputerAlwaysAllowed(target, enabled)}
                  />
                </SettingsRow>
              );
            }}
          </For>
        </SettingsCard>
      </SettingsSection>

      <Show when={pluginRuntimeEnabled()}>
        <SettingsSection
          title={zh() ? "本地 Plugins 与 Connectors" : "Local Plugins and Connectors"}
        >
          <SettingsCard>
            <SettingsRow
              label={zh() ? "导入插件" : "Import plugin"}
              description={
                zh()
                  ? "目录和 ZIP Bundle 都会校验 manifest、边界、哈希和贡献点。"
                  : "Directories and ZIP bundles validate manifests, boundaries, hashes, and contributions."
              }
            >
              <div class="local-hosts-actions">
                <Button
                  disabled={Boolean(busy()) || !props.featureFlags.pluginRuntime}
                  onClick={() => void installPlugin(false)}
                >
                  {zh() ? "选择目录" : "Choose folder"}
                </Button>
                <Button
                  disabled={Boolean(busy()) || !props.featureFlags.pluginRuntime}
                  onClick={() => void installPlugin(true)}
                >
                  {zh() ? "选择 Bundle" : "Choose bundle"}
                </Button>
                <Button
                  disabled={Boolean(busy()) || !props.featureFlags.pluginRuntime}
                  data-testid="local-hosts-install-sample-crm"
                  onClick={() => void installBuiltinSampleCrm()}
                >
                  {zh() ? "安装内置 sample-crm" : "Install built-in sample-crm"}
                </Button>
                <Show when={enterpriseEnabled()}>
                  <For each={["wecom", "ding_talk", "feishu"] as EnterprisePlatformId[]}>
                    {(platform) => (
                      <Button
                        disabled={Boolean(busy()) || !props.featureFlags.pluginRuntime}
                        data-testid={`local-hosts-install-${platform}`}
                        onClick={() => void installBuiltinEnterprise(platform)}
                      >
                        {zh() ? `安装 ${platform}` : `Install ${platform}`}
                      </Button>
                    )}
                  </For>
                </Show>
              </div>
            </SettingsRow>
            <For
              each={plugins()}
              fallback={
                <SettingsRow label={zh() ? "已安装插件" : "Installed plugins"}>
                  <span class="local-hosts-empty">{zh() ? "暂无" : "None"}</span>
                </SettingsRow>
              }
            >
              {(plugin) => (
                <SettingsRow
                  label={`${plugin.manifest.name} ${plugin.manifest.version}`}
                  description={`${plugin.manifest.id} · ${
                    pluginContributions()
                      .filter((entry) => entry.pluginId === plugin.manifest.id)
                      .map((entry) => `${entry.kind}:${entry.contributionId}=${entry.state}`)
                      .join(", ") || `${plugin.manifest.contributions?.length ?? 0} registered`
                  } · ${pluginPermissionDiffs()[plugin.manifest.id]?.addedScopes.length ? `${zh() ? "新增权限" : "added scopes"}: ${pluginPermissionDiffs()[plugin.manifest.id]!.addedScopes.join(", ")}` : plugin.diagnostics.join("; ") || plugin.contentHash.slice(0, 12)} · ${pluginLifecycle()[plugin.manifest.id]?.[0]?.status ?? "no_lifecycle"}`}
                >
                  <div
                    class="local-hosts-actions"
                    data-testid={`local-hosts-plugin-${plugin.manifest.id}`}
                  >
                    <Badge
                      tone={
                        plugin.status === "enabled"
                          ? "success"
                          : plugin.status === "needs_attention"
                            ? "warning"
                            : "neutral"
                      }
                    >
                      {plugin.status}
                    </Badge>
                    <Toggle
                      label={zh() ? "启用插件" : "Enable plugin"}
                      checked={plugin.status === "enabled"}
                      disabled={Boolean(busy()) || plugin.status === "invalid"}
                      onChange={() => void mutatePlugin(plugin, "toggle")}
                    />
                    <For
                      each={pluginContributions().filter(
                        (entry) =>
                          entry.pluginId === plugin.manifest.id && entry.kind === "custom_ui",
                      )}
                    >
                      {(entry) => (
                        <Button
                          size="small"
                          disabled={
                            Boolean(busy()) ||
                            plugin.status !== "enabled" ||
                            entry.state !== "active"
                          }
                          data-testid={`local-hosts-open-plugin-ui-${entry.contributionId}`}
                          onClick={() =>
                            void openCustomUi(plugin.manifest.id, entry.contributionId)
                          }
                        >
                          {zh() ? "打开界面" : "Open UI"}
                        </Button>
                      )}
                    </For>
                    <Button
                      size="small"
                      disabled={Boolean(busy())}
                      onClick={() => void mutatePlugin(plugin, "health")}
                    >
                      {zh() ? "检查" : "Check"}
                    </Button>
                    <Button
                      size="small"
                      disabled={
                        Boolean(busy()) ||
                        !pluginRevisions()[plugin.manifest.id]?.some(
                          (revision) => revision.status === "superseded",
                        )
                      }
                      onClick={() => void mutatePlugin(plugin, "rollback")}
                    >
                      {zh() ? "回滚上一版本" : "Rollback previous"}
                    </Button>
                    <Button
                      size="small"
                      variant="danger"
                      disabled={Boolean(busy())}
                      onClick={() => void mutatePlugin(plugin, "remove")}
                    >
                      {zh() ? "卸载" : "Remove"}
                    </Button>
                  </div>
                </SettingsRow>
              )}
            </For>
            <For
              each={pluginContributions().filter((entry) =>
                ["hook", "asset", "custom_ui"].includes(entry.kind),
              )}
            >
              {(entry) => {
                const surface = () => pluginSurfaces()[`${entry.pluginId}:${entry.contributionId}`];
                return (
                  <SettingsRow
                    label={`${entry.kind}:${entry.contributionId}`}
                    description={`${entry.pluginId} · ${entry.runtimeRevision.slice(0, 12)} · ${surface()?.lastResultCode ?? entry.diagnostic ?? "no_error"}`}
                  >
                    <Badge
                      tone={entry.state === "active" ? "success" : "warning"}
                      data-testid={`local-hosts-contribution-${entry.contributionId}`}
                    >
                      {entry.state}
                    </Badge>
                  </SettingsRow>
                );
              }}
            </For>
            <Show when={openPluginSurface()}>
              {(surface) => (
                <div class="local-hosts-plugin-ui" data-testid="local-hosts-plugin-ui">
                  <div class="local-hosts-plugin-ui-header">
                    <div>
                      <strong>{surface().pluginId}</strong>
                      <span>{surface().contributionId}</span>
                    </div>
                    <Button size="small" onClick={closeCustomUi}>
                      {zh() ? "关闭" : "Close"}
                    </Button>
                  </div>
                  <iframe
                    ref={pluginFrame}
                    title={`${surface().pluginId} ${surface().contributionId}`}
                    src={surface().entryUrl ?? undefined}
                    sandbox="allow-scripts"
                    data-testid="local-hosts-plugin-ui-frame"
                  />
                </div>
              )}
            </Show>
            <SettingsRow
              label="sample-crm Connector"
              description={
                zh()
                  ? "确定性本地账户；Host identity、Schema 和 Action revision 由已安装贡献点计算。"
                  : "Deterministic local account; Host identity, Schema, and Action revisions are computed from the installed contribution."
              }
            >
              <div class="local-hosts-actions">
                <TextField
                  label={zh() ? "账户名称" : "Account name"}
                  value={connectorName()}
                  testId="local-hosts-connector-name"
                  onInput={(event) => setConnectorName(event.currentTarget.value)}
                />
                <TextField
                  label={zh() ? "可选凭据" : "Optional credential"}
                  type="password"
                  value={connectorSecret()}
                  testId="local-hosts-connector-secret"
                  onInput={(event) => setConnectorSecret(event.currentTarget.value)}
                />
                <Button
                  disabled={Boolean(busy())}
                  data-testid="local-hosts-create-sample-account"
                  onClick={() => void createSampleAccount()}
                >
                  {zh() ? "创建账户" : "Create account"}
                </Button>
              </div>
            </SettingsRow>
            <Show when={enterpriseEnabled()}>
              <SettingsRow
                label={zh() ? "企业 Connector" : "Enterprise Connector"}
                description={
                  zh()
                    ? "首期支持企业微信、钉钉、飞书。凭据必须使用对应平台的 JSON 格式，正文不会进入 SQLite。"
                    : "The first release supports WeCom, DingTalk, and Feishu. Credentials use platform-specific JSON and never enter SQLite."
                }
              >
                <div class="local-hosts-actions">
                  <Select
                    label={zh() ? "平台" : "Platform"}
                    value={enterprisePlatform()}
                    options={[
                      { value: "wecom", label: zh() ? "企业微信" : "WeCom" },
                      { value: "ding_talk", label: zh() ? "钉钉" : "DingTalk" },
                      { value: "feishu", label: zh() ? "飞书" : "Feishu" },
                    ]}
                    onChange={(value) => setEnterprisePlatform(value as EnterprisePlatformId)}
                  />
                  <TextField
                    label={zh() ? "账户名称" : "Account name"}
                    value={enterpriseConnectorName()}
                    onInput={(event) => setEnterpriseConnectorName(event.currentTarget.value)}
                  />
                  <TextField
                    label={zh() ? "凭据 JSON" : "Credential JSON"}
                    type="password"
                    value={enterpriseCredential()}
                    onInput={(event) => setEnterpriseCredential(event.currentTarget.value)}
                  />
                  <Button disabled={Boolean(busy())} onClick={() => void createEnterpriseAccount()}>
                    {zh() ? "创建企业账户" : "Create enterprise account"}
                  </Button>
                </div>
              </SettingsRow>
            </Show>
            <For each={connectorAccounts()}>
              {(account) => (
                <SettingsRow
                  label={account.displayName}
                  description={`${account.pluginId}:${account.connectorId} · ${account.revision.hostIdentityHash.slice(0, 12)} · ${account.revision.schemaHash.slice(0, 12)} · ${account.revision.actionHash.slice(0, 12)}`}
                >
                  <div
                    class="local-hosts-actions"
                    data-testid={`local-hosts-connector-${account.id}`}
                  >
                    <Badge tone={account.health === "healthy" ? "success" : "warning"}>
                      {account.health}
                    </Badge>
                    <Button
                      size="small"
                      variant="danger"
                      disabled={Boolean(busy()) || account.health === "revoked"}
                      onClick={() => void revokeConnector(account)}
                    >
                      {zh() ? "撤销凭据" : "Revoke"}
                    </Button>
                  </div>
                </SettingsRow>
              )}
            </For>
          </SettingsCard>
        </SettingsSection>
      </Show>

      <SettingsSection title={zh() ? "Channel 与 Gateway" : "Channel and Gateway"}>
        <SettingsCard>
          <SettingsRow
            label={zh() ? "Provider 账户与路由" : "Provider account and route"}
            description={
              zh()
                ? "凭据只进入系统凭据库；SQLite 仅保存 secret_ref 与精确 peer/thread allowlist。"
                : "Credentials go only to the OS credential store; SQLite keeps only secret_ref and the exact peer/thread allowlist."
            }
          >
            <div class="local-hosts-actions local-hosts-channel-editor">
              <Select
                label="Provider"
                value={channelProviderId()}
                options={channelManifests().map((manifest) => ({
                  value: manifest.id,
                  label: manifest.id,
                  description: manifest.runtimeKind,
                }))}
                placeholder={zh() ? "选择 Provider" : "Select provider"}
                disabled={Boolean(busy())}
                onChange={setChannelProviderId}
              />
              <TextField
                label={zh() ? "账户 ID" : "Account ID"}
                value={channelAccountId()}
                onInput={(event) => {
                  setChannelAccountId(event.currentTarget.value);
                  setChannelExpectedRevision(undefined);
                }}
              />
              <TextField
                label={zh() ? "显示名称" : "Display name"}
                value={channelDisplayName()}
                onInput={(event) => setChannelDisplayName(event.currentTarget.value)}
              />
              <TextField
                label={zh() ? "凭据（留空则保留）" : "Credential (blank keeps existing)"}
                type="password"
                value={channelCredential()}
                onInput={(event) => setChannelCredential(event.currentTarget.value)}
              />
              <TextField
                label="Peer"
                value={channelPeer()}
                onInput={(event) => setChannelPeer(event.currentTarget.value)}
              />
              <TextField
                label="Thread"
                value={channelThread()}
                onInput={(event) => setChannelThread(event.currentTarget.value)}
              />
              <Button disabled={Boolean(busy())} onClick={() => void saveChannelAccount()}>
                {zh() ? "保存账户" : "Save account"}
              </Button>
            </div>
          </SettingsRow>
          <For each={channelProviders()}>
            {(provider) => (
              <SettingsRow
                label={provider.providerId}
                description={`${zh() ? "配置 revision" : "Config revision"} ${provider.configRevision} · ${provider.diagnostic ?? "no_error"}`}
              >
                <Badge tone={provider.state === "healthy" ? "success" : "warning"}>
                  {provider.state}
                </Badge>
              </SettingsRow>
            )}
          </For>
          <For each={channelAccounts()}>
            {(account) => (
              <SettingsRow
                label={account.displayName}
                description={`${account.providerId} · revision ${account.configRevision} · ${account.routeAllowlist.length} route${account.routeAllowlist.length === 1 ? "" : "s"}`}
              >
                <div class="local-hosts-actions">
                  <Toggle
                    label={zh() ? "启用 Channel 账户" : "Enable Channel account"}
                    checked={account.enabled}
                    disabled={Boolean(busy())}
                    onChange={(enabled) => void setChannelAccountEnabled(account, enabled)}
                  />
                  <Button
                    size="small"
                    disabled={Boolean(busy())}
                    onClick={() => editChannelAccount(account)}
                  >
                    {zh() ? "编辑" : "Edit"}
                  </Button>
                </div>
              </SettingsRow>
            )}
          </For>
          <SettingsRow
            label={zh() ? "普通用户登录启动" : "Per-user login startup"}
            description={
              gateway()
                ? `${gateway()!.channels.join(", ")} · ingress ${gateway()!.pendingIngress} · outbox ${gateway()!.pendingDeliveries}`
                : zh()
                  ? "正在读取 Gateway health"
                  : "Loading Gateway health"
            }
          >
            <Toggle
              label={zh() ? "登录后启动 Gateway" : "Start Gateway after login"}
              testId="local-hosts-gateway-startup"
              checked={gateway()?.startupRegistered ?? false}
              disabled={Boolean(busy()) || !props.featureFlags.localGateway}
              onChange={(enabled) => void updateGatewayStartup(enabled)}
            />
          </SettingsRow>
          <SettingsRow
            label={zh() ? "持久 ledger 恢复" : "Durable ledger recovery"}
            description={
              zh()
                ? "重新 claim ingress/outbox，不重复创建已绑定 Run 或已成功投递。"
                : "Reclaims ingress/outbox without recreating bound Runs or delivered messages."
            }
          >
            <div class="local-hosts-actions">
              <Button
                disabled={Boolean(busy()) || !props.featureFlags.localGateway}
                data-testid="local-hosts-gateway-reconcile"
                onClick={() => void reconcileGateway()}
              >
                {zh() ? "立即 reconcile" : "Reconcile now"}
              </Button>
              <Button
                disabled={Boolean(busy()) || !props.featureFlags.localGateway}
                data-testid="local-hosts-mock-poll"
                onClick={() => void runMockPollSample()}
              >
                {zh() ? "运行 mock-poll" : "Run mock-poll"}
              </Button>
            </div>
          </SettingsRow>
        </SettingsCard>
      </SettingsSection>
    </div>
  );
}
