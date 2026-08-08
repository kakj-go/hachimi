import {
  commandFailure,
  commands,
  type ChannelAccessPolicy,
  type ChannelAuthorization,
  type ChannelAuthorizationTarget,
  type ChannelDmPolicy,
  type ChannelGrant,
  type ChannelGroupHistoryPolicy,
  type ChannelIdentityLinkCode,
  type ChannelIdentityTransferPreview,
  type ChannelMentionPolicy,
  type ChannelPairingCode,
  type ChannelTopicPolicy,
  type GatewayHealth,
  type IntegrationCredentialInput,
  type IntegrationProviderAccount,
  type IntegrationProviderDefinition,
  type IntegrationProviderId,
  type IlinkQrSession,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  Badge,
  Button,
  Dialog,
  KeyRound,
  Link,
  NativeSelect,
  PageHeading,
  Plus,
  RefreshCw,
  SettingsCard,
  SettingsRow,
  SettingsSection,
  ShieldCheck,
  StatusBanner,
  Switch as Toggle,
  Tabs,
  TextField,
  Trash2,
  Users,
} from "@hachimi/ui";
import { For, Show, createMemo, createSignal, onCleanup, onMount, untrack } from "solid-js";

import { AuthorizationEditor } from "./platform-integration-authorization-editor";
import { ChannelGrantEditor } from "./platform-integration-grant-editor";
import { integrationFailureMessage } from "./platform-integration-errors";
import { IntegrationAccountWizard } from "./platform-integration-wizard";
import { createPermissionPolicy } from "./permission-policy-editor";
import { PermissionScopeConfirmation } from "./permission-scope-confirmation";
import { permissionScopeRisk } from "./permission-scope-risk";
import { RuntimeHealthBanner } from "./runtime-health";

type AccountDialogMode = "create" | "credentials" | "capabilities";

interface AccountDraft {
  mode: AccountDialogMode;
  accountId: string;
  displayName: string;
  apiAccess: boolean;
  messaging: boolean;
  expectedRevision: number | null;
  fields: Record<string, string>;
}

interface PairingDraft {
  target: ChannelAuthorizationTarget;
  groupHistoryPolicy: ChannelGroupHistoryPolicy | null;
  topicPolicy: ChannelTopicPolicy | null;
  mentionPolicy: ChannelMentionPolicy | null;
  grant: ChannelGrant;
}

interface IlinkQrDialogState {
  session: IlinkQrSession;
  displayName: string;
}

const EMPTY_GRANT: ChannelGrant = {
  permissionPolicy: createPermissionPolicy(),
  skillIds: [],
  mcpServerIds: [],
  connectorSelections: [],
  readOnlyWorkspaceRoots: [],
  networkHosts: [],
};

export function PlatformIntegrationsSettings() {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const [providers, setProviders] = createSignal<IntegrationProviderDefinition[]>([]);
  const [accounts, setAccounts] = createSignal<IntegrationProviderAccount[]>([]);
  const [gateway, setGateway] = createSignal<GatewayHealth>();
  const [selected, setSelected] = createSignal<IntegrationProviderId>("dingtalk");
  const [draft, setDraft] = createSignal<AccountDraft>();
  const [wizardProvider, setWizardProvider] = createSignal<IntegrationProviderDefinition>();
  const [pairingAccount, setPairingAccount] = createSignal<IntegrationProviderAccount>();
  const [policyAccount, setPolicyAccount] = createSignal<IntegrationProviderAccount>();
  const [accessPolicy, setAccessPolicy] = createSignal<ChannelAccessPolicy>();
  const [authorizationAccount, setAuthorizationAccount] =
    createSignal<IntegrationProviderAccount>();
  const [disconnectAccount, setDisconnectAccount] = createSignal<IntegrationProviderAccount>();
  const [authorizations, setAuthorizations] = createSignal<ChannelAuthorization[]>([]);
  const [identityLinkCode, setIdentityLinkCode] = createSignal<ChannelIdentityLinkCode>();
  const [authorizationEditor, setAuthorizationEditor] = createSignal<{
    account: IntegrationProviderAccount;
    value?: ChannelAuthorization;
  }>();
  const [transferAccount, setTransferAccount] = createSignal<IntegrationProviderAccount>();
  const [transferPreviews, setTransferPreviews] = createSignal<ChannelIdentityTransferPreview[]>(
    [],
  );
  const [ilinkQr, setIlinkQr] = createSignal<IlinkQrDialogState>();
  const [loading, setLoading] = createSignal(false);
  const [submitting, setSubmitting] = createSignal(false);
  const [probing, setProbing] = createSignal<string>();
  const [failure, setFailure] = createSignal<string>();
  const [dialogFailure, setDialogFailure] = createSignal<string>();
  const [notice, setNotice] = createSignal<string>();
  const activeProvider = createMemo(() =>
    providers().find((provider) => provider.id === selected()),
  );

  async function load() {
    setLoading(true);
    setFailure(undefined);
    try {
      const [nextProviders, nextAccounts, nextGateway] = await Promise.all([
        commands.listIntegrationProviders(),
        commands.listEnterpriseIntegrations(),
        commands.getGatewayHealth(),
      ]);
      setProviders(nextProviders);
      setAccounts(nextAccounts);
      setGateway(nextGateway);
      if (!nextProviders.some((provider) => provider.id === selected())) {
        setSelected(nextProviders[0]?.id ?? "dingtalk");
      }
    } catch (error) {
      setFailure(integrationFailureMessage(error, zh()));
    } finally {
      setLoading(false);
    }
  }

  function openCreate(provider: IntegrationProviderDefinition) {
    setWizardProvider(provider);
  }

  function openAccount(account: IntegrationProviderAccount, mode: AccountDialogMode) {
    const provider = providers().find((candidate) => candidate.id === account.providerId);
    if (!provider) return;
    setSelected(account.providerId);
    setDialogFailure(undefined);
    setDraft({
      mode,
      accountId: account.id,
      displayName: account.displayName,
      apiAccess: account.apiAccessEnabled,
      messaging: account.messagingEnabled,
      expectedRevision: account.configRevision,
      fields: Object.fromEntries(provider.credentialFields.map((field) => [field.id, ""])),
    });
  }

  async function saveDraft(provider: IntegrationProviderDefinition) {
    const current = draft();
    if (!current) return;
    const invalid = validateDraft(provider, current, zh());
    if (invalid) {
      setDialogFailure(invalid);
      return;
    }
    setSubmitting(true);
    setDialogFailure(undefined);
    try {
      if (provider.id === "wechat_ilink" && current.mode !== "capabilities") {
        const session = await commands.beginIlinkQrLogin({
          accountId: current.accountId,
          displayName: current.displayName.trim(),
        });
        setIlinkQr({ session, displayName: current.displayName.trim() });
      } else if (current.mode === "capabilities") {
        await commands.setEnterpriseIntegrationCapabilities({
          id: current.accountId,
          apiAccessEnabled: current.apiAccess,
          messagingEnabled: current.messaging,
          expectedConfigRevision: current.expectedRevision ?? 0,
        });
      } else {
        await commands.upsertEnterpriseIntegration({
          id: current.accountId,
          displayName: current.displayName.trim(),
          credential: credential(provider.id, current.fields),
          apiAccessEnabled: current.apiAccess,
          messagingEnabled: current.messaging,
          expectedConfigRevision: current.expectedRevision,
        });
      }
      setDraft(undefined);
      if (provider.id !== "wechat_ilink" || current.mode === "capabilities") {
        setNotice(zh() ? "平台账户已更新。" : "Platform account updated.");
        await load();
      }
    } catch (error) {
      setDialogFailure(integrationFailureMessage(error, zh()));
    } finally {
      setSubmitting(false);
    }
  }

  async function probe(account: IntegrationProviderAccount) {
    setProbing(account.id);
    setFailure(undefined);
    try {
      const result = await commands.probeEnterpriseIntegration(account.id);
      const healthy = [result.credential, result.ingress, result.egress, result.api].every(
        (dimension) => dimension.ok,
      );
      setNotice(
        healthy
          ? zh()
            ? `${account.displayName} 验证通过。`
            : `${account.displayName} verified.`
          : zh()
            ? `${account.displayName} 需要处理。`
            : `${account.displayName} needs attention.`,
      );
      await load();
    } catch (error) {
      setFailure(integrationFailureMessage(error, zh()));
    } finally {
      setProbing(undefined);
    }
  }

  async function openAuthorizations(account: IntegrationProviderAccount) {
    setDialogFailure(undefined);
    setAuthorizationAccount(account);
    try {
      setAuthorizations(await commands.listChannelAuthorizations(account.id));
    } catch (error) {
      setDialogFailure(commandFailure(error).message);
    }
  }

  async function openTransfers(account: IntegrationProviderAccount) {
    setDialogFailure(undefined);
    setTransferAccount(account);
    try {
      setTransferPreviews(await commands.listChannelIdentityTransferPreviews(account.id));
    } catch (error) {
      setDialogFailure(commandFailure(error).message);
    }
  }

  async function confirmTransfer(preview: ChannelIdentityTransferPreview) {
    setSubmitting(true);
    setDialogFailure(undefined);
    try {
      await commands.transferChannelIdentity({
        id: preview.id,
        expectedRevision: preview.revision,
        expectedSourceGroupRevision: preview.sourceGroupRevision,
        expectedTargetGroupRevision: preview.targetGroupRevision,
      });
      setTransferPreviews((values) => values.filter((value) => value.id !== preview.id));
      setNotice(zh() ? "身份已转移到新的共享会话。" : "Identity moved to a new shared Session.");
    } catch (error) {
      setDialogFailure(commandFailure(error).message);
    } finally {
      setSubmitting(false);
    }
  }

  async function openPolicy(account: IntegrationProviderAccount) {
    setDialogFailure(undefined);
    setPolicyAccount(account);
    setAccessPolicy(undefined);
    try {
      setAccessPolicy(await commands.getChannelAccessPolicy(account.id));
    } catch (error) {
      setDialogFailure(commandFailure(error).message);
    }
  }

  async function createIdentityLinkCode(authorization: ChannelAuthorization) {
    const actorId = authorization.actorId;
    if (!actorId) return;
    setDialogFailure(undefined);
    try {
      setIdentityLinkCode(
        await commands.createChannelIdentityLinkCode({
          accountId: authorization.accountId,
          actorId,
        }),
      );
      setAuthorizationAccount(undefined);
    } catch (error) {
      setDialogFailure(commandFailure(error).message);
    }
  }

  async function pollIlinkQr() {
    const current = ilinkQr();
    if (!current) return;
    try {
      const session = await commands.pollIlinkQrLogin(current.session.accountId);
      setIlinkQr({ ...current, session });
      if (session.state === "confirmed") {
        setNotice(zh() ? "微信 iLink 已连接。" : "WeChat iLink connected.");
        setIlinkQr(undefined);
        await load();
      }
    } catch (error) {
      setDialogFailure(commandFailure(error).message);
    }
  }

  async function refreshIlinkQr() {
    const current = ilinkQr();
    if (!current) return;
    try {
      const session = await commands.beginIlinkQrLogin({
        accountId: current.session.accountId,
        displayName: current.displayName,
      });
      setIlinkQr({ ...current, session });
    } catch (error) {
      setDialogFailure(commandFailure(error).message);
    }
  }

  async function cancelIlinkQr() {
    const current = ilinkQr();
    if (!current) return;
    try {
      await commands.cancelIlinkQrLogin(current.session.accountId);
      setIlinkQr(undefined);
      await load();
    } catch (error) {
      setDialogFailure(commandFailure(error).message);
    }
  }

  async function removeAccount() {
    const account = disconnectAccount();
    if (!account) return;
    setSubmitting(true);
    try {
      await commands.removeEnterpriseIntegration(account.id);
      setDisconnectAccount(undefined);
      await load();
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setSubmitting(false);
    }
  }

  onMount(() => void load());

  return (
    <div class="settings-page platform-integrations-page" data-testid="settings-integrations-page">
      <PageHeading
        class="settings-page-heading"
        title={zh() ? "平台集成" : "Platform integrations"}
        description={
          zh()
            ? "消息 Channel、企业 API 与会话授权"
            : "Channels, enterprise APIs, and conversation access"
        }
        actions={
          <Button
            aria-label={zh() ? "刷新平台集成" : "Refresh integrations"}
            disabled={loading()}
            onClick={() => void load()}
          >
            <RefreshCw size={15} />
          </Button>
        }
      />

      <Show when={failure()}>
        {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
      </Show>
      <Show when={notice()}>
        {(message) => <StatusBanner tone="success">{message()}</StatusBanner>}
      </Show>

      <SettingsSection title={zh() ? "Gateway" : "Gateway"}>
        <RuntimeHealthBanner component="gateway" zh={zh()} />
        <SettingsCard>
          <SettingsRow
            label={zh() ? "本地消息服务" : "Local messaging service"}
            description={
              gateway()?.lastHeartbeatMs
                ? new Date(gateway()!.lastHeartbeatMs!).toLocaleString()
                : ""
            }
          >
            <Badge tone={gateway()?.running ? "success" : "neutral"}>
              {gateway()?.running ? (zh() ? "运行中" : "Running") : zh() ? "未运行" : "Stopped"}
            </Badge>
          </SettingsRow>
        </SettingsCard>
      </SettingsSection>

      <Show when={providers().length > 0}>
        <div class="integration-provider-tabs">
          <Tabs
            value={selected()}
            onChange={(value) => setSelected(value as IntegrationProviderId)}
            tabs={providers().map((provider) => ({
              value: provider.id,
              ariaLabel: zh() ? provider.nameZh : provider.nameEn,
              label: <ProviderLabel provider={provider} zh={zh()} />,
              content: (
                <ProviderPanel
                  provider={provider}
                  accounts={accounts().filter((account) => account.providerId === provider.id)}
                  busy={loading() || submitting()}
                  probing={probing()}
                  zh={zh()}
                  onCreate={() => openCreate(provider)}
                  onSettings={(account) => openAccount(account, "capabilities")}
                  onCredentials={(account) => openAccount(account, "credentials")}
                  onProbe={(account) => void probe(account)}
                  onPairing={setPairingAccount}
                  onPolicy={(account) => void openPolicy(account)}
                  onAuthorizations={(account) => void openAuthorizations(account)}
                  onTransfers={(account) => void openTransfers(account)}
                  onRemove={setDisconnectAccount}
                />
              ),
            }))}
          />
        </div>
      </Show>

      <Show when={wizardProvider()}>
        {(provider) => (
          <IntegrationAccountWizard
            provider={provider()}
            zh={zh()}
            onClose={() => setWizardProvider(undefined)}
            onCompleted={(account, probeFailed) => {
              setWizardProvider(undefined);
              setNotice(
                probeFailed
                  ? zh()
                    ? `${account.displayName} 已创建，自动检测未通过，可在卡片中重新检测。`
                    : `${account.displayName} was created; automatic checks did not pass. Retry from the card.`
                  : zh()
                    ? `${account.displayName} 已连接并完成检测。`
                    : `${account.displayName} connected and checked.`,
              );
              void load();
            }}
          />
        )}
      </Show>

      <Show when={draft() && activeProvider()}>
        <Dialog
          open
          size="wide"
          title={dialogTitle(draft()!.mode, activeProvider()!, zh())}
          closeLabel={zh() ? "关闭" : "Close"}
          loading={submitting()}
          invalid={Boolean(dialogFailure())}
          onOpenChange={(open) => !open && !submitting() && setDraft(undefined)}
        >
          <AccountForm
            provider={activeProvider()!}
            draft={draft()!}
            busy={submitting()}
            failure={dialogFailure()}
            zh={zh()}
            onPatch={(update) =>
              setDraft((current) => (current ? { ...current, ...update } : current))
            }
            onField={(id, value) =>
              setDraft((current) =>
                current ? { ...current, fields: { ...current.fields, [id]: value } } : current,
              )
            }
            onCancel={() => setDraft(undefined)}
            onSave={() => void saveDraft(activeProvider()!)}
          />
        </Dialog>
      </Show>

      <Show when={pairingAccount()}>
        {(account) => (
          <PairingDialog
            account={account()}
            provider={providers().find((provider) => provider.id === account().providerId)!}
            zh={zh()}
            onClose={() => setPairingAccount(undefined)}
          />
        )}
      </Show>

      <Show when={policyAccount() && accessPolicy()}>
        <PolicyDialog
          policy={accessPolicy()!}
          zh={zh()}
          onClose={() => {
            setPolicyAccount(undefined);
            setAccessPolicy(undefined);
          }}
          onSaved={(policy) => {
            setAccessPolicy(policy);
            setNotice(zh() ? "消息策略已更新。" : "Messaging policy updated.");
          }}
        />
      </Show>

      <Show when={authorizationAccount()}>
        {(account) => (
          <Dialog
            open
            title={zh() ? "会话授权" : "Conversation access"}
            closeLabel={zh() ? "关闭" : "Close"}
            onOpenChange={(open) => !open && setAuthorizationAccount(undefined)}
          >
            <AuthorizationList
              values={authorizations()}
              zh={zh()}
              failure={dialogFailure()}
              onLink={(authorization) => void createIdentityLinkCode(authorization)}
              onAdd={() => setAuthorizationEditor({ account: account() })}
              onEdit={(authorization) =>
                setAuthorizationEditor({ account: account(), value: authorization })
              }
            />
          </Dialog>
        )}
      </Show>

      <Show when={authorizationEditor()}>
        {(editor) => (
          <AuthorizationEditor
            account={editor().account}
            provider={providers().find((provider) => provider.id === editor().account.providerId)!}
            {...(editor().value ? { value: editor().value } : {})}
            zh={zh()}
            onClose={() => setAuthorizationEditor(undefined)}
            onSaved={(authorization) => {
              setAuthorizations((values) => [
                ...values.filter((value) => value.id !== authorization.id),
                authorization,
              ]);
              setNotice(zh() ? "会话授权已更新。" : "Conversation access updated.");
            }}
          />
        )}
      </Show>

      <Show when={transferAccount()}>
        <Dialog
          open
          title={zh() ? "身份所有权冲突" : "Identity ownership conflicts"}
          closeLabel={zh() ? "关闭" : "Close"}
          loading={submitting()}
          onOpenChange={(open) => !open && !submitting() && setTransferAccount(undefined)}
        >
          <div class="integration-form integration-transfer-list">
            <Show
              when={transferPreviews().length > 0}
              fallback={
                <span class="integration-route-empty">
                  {zh() ? "暂无待确认冲突" : "No pending conflicts"}
                </span>
              }
            >
              <For each={transferPreviews()}>
                {(preview) => (
                  <div class="integration-authorization-row">
                    <span>
                      <strong>
                        {preview.source.displayName ?? preview.source.actorId} →{" "}
                        {preview.target.displayName ?? preview.target.actorId}
                      </strong>
                      <small>
                        {preview.source.providerId} / {preview.target.providerId}
                      </small>
                    </span>
                    <Button
                      variant="primary"
                      disabled={submitting()}
                      onClick={() => void confirmTransfer(preview)}
                    >
                      {zh() ? "确认转移" : "Confirm transfer"}
                    </Button>
                  </div>
                )}
              </For>
            </Show>
            <Show when={dialogFailure()}>
              {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
            </Show>
            <div class="integration-form-actions">
              <Button onClick={() => setTransferAccount(undefined)}>
                {zh() ? "关闭" : "Close"}
              </Button>
            </div>
          </div>
        </Dialog>
      </Show>

      <Show when={identityLinkCode()}>
        {(linkCode) => (
          <Dialog
            open
            title={zh() ? "身份关联码" : "Identity link code"}
            closeLabel={zh() ? "关闭" : "Close"}
            onOpenChange={(open) => !open && setIdentityLinkCode(undefined)}
          >
            <div class="integration-form">
              <div class="integration-pairing-code">
                <code>{linkCode().code}</code>
                <span>{new Date(linkCode().expiresAtMs).toLocaleTimeString()}</span>
              </div>
              <div class="integration-form-actions">
                <Button onClick={() => setIdentityLinkCode(undefined)}>
                  {zh() ? "完成" : "Done"}
                </Button>
              </div>
            </div>
          </Dialog>
        )}
      </Show>

      <Show when={ilinkQr()}>
        {(value) => (
          <IlinkQrDialog
            session={value().session}
            zh={zh()}
            failure={dialogFailure()}
            onPoll={() => void pollIlinkQr()}
            onRefresh={refreshIlinkQr}
            onCancel={() => void cancelIlinkQr()}
          />
        )}
      </Show>

      <Show when={disconnectAccount()}>
        {(account) => (
          <Dialog
            open
            title={zh() ? `断开 ${account().displayName}` : `Disconnect ${account().displayName}`}
            description={
              zh()
                ? "本地凭据、授权和会话绑定将被移除。"
                : "Local credentials, access, and bindings will be removed."
            }
            closeLabel={zh() ? "关闭" : "Close"}
            tone="danger"
            loading={submitting()}
            onOpenChange={(open) => !open && !submitting() && setDisconnectAccount(undefined)}
          >
            <div class="integration-form-actions integration-disconnect-dialog">
              <Button disabled={submitting()} onClick={() => setDisconnectAccount(undefined)}>
                {zh() ? "取消" : "Cancel"}
              </Button>
              <Button
                variant="danger"
                data-testid="integration-disconnect-confirm"
                disabled={submitting()}
                onClick={() => void removeAccount()}
              >
                <Trash2 size={15} /> {zh() ? "断开连接" : "Disconnect"}
              </Button>
            </div>
          </Dialog>
        )}
      </Show>
    </div>
  );
}

function ProviderLabel(props: { provider: IntegrationProviderDefinition; zh: boolean }) {
  return (
    <span class="integration-tab-label">
      <img src={`/${props.provider.iconAsset}`} alt="" />
      <span>{props.zh ? props.provider.nameZh : props.provider.nameEn}</span>
    </span>
  );
}

function ProviderPanel(props: {
  provider: IntegrationProviderDefinition;
  accounts: IntegrationProviderAccount[];
  busy: boolean;
  probing: string | undefined;
  zh: boolean;
  onCreate: () => void;
  onSettings: (account: IntegrationProviderAccount) => void;
  onCredentials: (account: IntegrationProviderAccount) => void;
  onProbe: (account: IntegrationProviderAccount) => void;
  onPairing: (account: IntegrationProviderAccount) => void;
  onPolicy: (account: IntegrationProviderAccount) => void;
  onAuthorizations: (account: IntegrationProviderAccount) => void;
  onTransfers: (account: IntegrationProviderAccount) => void;
  onRemove: (account: IntegrationProviderAccount) => void;
}) {
  return (
    <div
      class="integration-provider-panel"
      data-testid={`integration-provider-${props.provider.id}`}
    >
      <div class="integration-provider-toolbar">
        <div>
          <strong>{props.zh ? props.provider.nameZh : props.provider.nameEn}</strong>
          <span>{transportLabel(props.provider.transport, props.zh)}</span>
        </div>
        <Button
          data-testid={`integration-connect-${props.provider.id}`}
          disabled={props.busy}
          onClick={props.onCreate}
        >
          <Plus size={15} /> {props.zh ? "连接账户" : "Connect account"}
        </Button>
      </div>
      <Show
        when={props.accounts.length > 0}
        fallback={
          <EmptyProvider provider={props.provider} zh={props.zh} onCreate={props.onCreate} />
        }
      >
        <div class="integration-account-grid">
          <For each={props.accounts}>
            {(account) => (
              <article
                class="integration-account-card"
                data-testid={`integration-account-${account.id}`}
              >
                <header class="integration-account-card-header">
                  <img src={`/${props.provider.iconAsset}`} alt="" />
                  <span>
                    <strong>{account.displayName}</strong>
                    <small>{account.diagnostic ?? capabilitySummary(account, props.zh)}</small>
                  </span>
                  <div class="integration-account-state">
                    <Badge tone={account.state === "healthy" ? "success" : "warning"}>
                      {stateLabel(account.state, props.zh)}
                    </Badge>
                    <Button
                      aria-label={props.zh ? "重新验证" : "Probe"}
                      disabled={props.busy || props.probing === account.id}
                      onClick={() => props.onProbe(account)}
                    >
                      <RefreshCw size={14} />
                    </Button>
                  </div>
                </header>
                <AccountDiagnostics account={account} provider={props.provider} zh={props.zh} />
                <footer class="integration-account-actions">
                  <Show when={account.messagingEnabled}>
                    <Button onClick={() => props.onPolicy(account)}>
                      <ShieldCheck size={14} /> {props.zh ? "策略与权限" : "Policy & access"}
                    </Button>
                    <Button onClick={() => props.onPairing(account)}>
                      <KeyRound size={14} /> {props.zh ? "连接码" : "Pairing"}
                    </Button>
                    <Button onClick={() => props.onAuthorizations(account)}>
                      <ShieldCheck size={14} /> {account.authorizations.length}
                    </Button>
                    <Button onClick={() => props.onTransfers(account)}>
                      <Users size={14} /> {props.zh ? "身份" : "Identities"}
                    </Button>
                  </Show>
                  <Button onClick={() => props.onSettings(account)}>
                    {props.zh ? "能力" : "Capabilities"}
                  </Button>
                  <Button onClick={() => props.onCredentials(account)}>
                    {props.zh ? "凭据" : "Credentials"}
                  </Button>
                  <Button
                    aria-label={props.zh ? "断开连接" : "Disconnect"}
                    data-testid={`integration-disconnect-${account.id}`}
                    variant="danger"
                    onClick={() => props.onRemove(account)}
                  >
                    <Trash2 size={14} />
                  </Button>
                </footer>
              </article>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

function AccountDiagnostics(props: {
  account: IntegrationProviderAccount;
  provider: IntegrationProviderDefinition;
  zh: boolean;
}) {
  const dimensions = () =>
    props.account.probe
      ? [
          ["credential", props.account.probe.credential] as const,
          ["ingress", props.account.probe.ingress] as const,
          ["egress", props.account.probe.egress] as const,
          ["api", props.account.probe.api] as const,
        ]
      : [];
  return (
    <div class="integration-account-detail">
      <Show when={dimensions().length > 0}>
        <div class="integration-probe-compact">
          <For each={dimensions()}>
            {(entry) => (
              <span data-ok={entry[1].ok} title={entry[1].diagnostic ?? entry[1].resultCode}>
                {entry[0]}
              </span>
            )}
          </For>
        </div>
      </Show>
      <span>
        {props.zh ? "最近接收" : "Last received"}:{" "}
        {formatTime(props.account.lastEventAtMs, props.zh)} · {props.zh ? "最近发送" : "Last sent"}:{" "}
        {formatTime(props.account.lastDeliveryAtMs, props.zh)}
      </span>
      <span>
        {props.zh ? "最近握手" : "Last handshake"}:{" "}
        {formatTime(props.account.lastHandshakeAtMs, props.zh)} ·{" "}
        {props.zh ? "最近帧" : "Last frame"}: {formatTime(props.account.lastFrameAtMs, props.zh)}
      </span>
      <Show when={props.account.lastErrorCode}>
        {(code) => <code>{providerRuntimeError(code(), props.zh)}</code>}
      </Show>
      <Show when={props.provider.id === "wecom_app"}>
        <code>/v1/channels/wecom_app/{props.account.id}/callback</code>
      </Show>
    </div>
  );
}

function EmptyProvider(props: {
  provider: IntegrationProviderDefinition;
  zh: boolean;
  onCreate: () => void;
}) {
  return (
    <div class="integration-empty-state">
      <img src={`/${props.provider.iconAsset}`} alt="" />
      <strong>{props.zh ? "尚未连接账户" : "No accounts connected"}</strong>
      <Button onClick={props.onCreate}>
        <Plus size={15} /> {props.zh ? "连接账户" : "Connect account"}
      </Button>
    </div>
  );
}

function AccountForm(props: {
  provider: IntegrationProviderDefinition;
  draft: AccountDraft;
  busy: boolean;
  failure: string | undefined;
  zh: boolean;
  onPatch: (update: Partial<AccountDraft>) => void;
  onField: (id: string, value: string) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const showCredentials = () => props.draft.mode !== "capabilities";
  const activeCredentialFields = () =>
    props.provider.credentialFields.filter((field) =>
      credentialFieldEnabled(field.capability, props.draft.apiAccess, props.draft.messaging),
    );
  return (
    <div class="integration-form integration-account-dialog">
      <ProviderLabel provider={props.provider} zh={props.zh} />
      <TextField
        label={props.zh ? "账户名称" : "Account name"}
        value={props.draft.displayName}
        disabled={props.busy}
        onInput={(event) => props.onPatch({ displayName: event.currentTarget.value })}
      />
      <Show when={showCredentials()}>
        <div class="integration-credential-grid">
          <For each={activeCredentialFields()}>
            {(field) => (
              <TextField
                label={field.label}
                value={props.draft.fields[field.id] ?? ""}
                type={
                  field.kind === "secret"
                    ? "password"
                    : field.kind === "integer"
                      ? "number"
                      : "text"
                }
                disabled={props.busy}
                onInput={(event) => props.onField(field.id, event.currentTarget.value)}
              />
            )}
          </For>
        </div>
      </Show>
      <div class="integration-capability-switches">
        <Show when={props.provider.capabilities.includes("api_access")}>
          <CapabilityToggle
            label={props.zh ? "企业 API" : "Enterprise API"}
            checked={props.draft.apiAccess}
            disabled={props.busy}
            onChange={(apiAccess) => props.onPatch({ apiAccess })}
          />
        </Show>
        <CapabilityToggle
          label={props.zh ? "消息 Channel" : "Messaging Channel"}
          checked={props.draft.messaging}
          disabled={props.busy}
          onChange={(messaging) => props.onPatch({ messaging })}
        />
      </div>
      <Show when={props.provider.id === "wecom_app" && props.draft.messaging}>
        <div class="integration-callback-path">
          /v1/channels/wecom_app/{props.draft.accountId}/callback
        </div>
      </Show>
      <Show when={props.provider.sourceStatus === "public_qualification_unverified"}>
        <StatusBanner tone="warning">
          {props.zh
            ? "公开资格与条款尚未独立核实"
            : "Public eligibility and terms are not independently verified"}
        </StatusBanner>
      </Show>
      <Show when={props.failure}>
        {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
      </Show>
      <div class="integration-form-actions">
        <Button disabled={props.busy} onClick={props.onCancel}>
          {props.zh ? "取消" : "Cancel"}
        </Button>
        <Button variant="primary" disabled={props.busy} onClick={props.onSave}>
          {props.zh ? "保存" : "Save"}
        </Button>
      </div>
    </div>
  );
}

function CapabilityToggle(props: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div class="integration-capability-option">
      <span>{props.label}</span>
      <Toggle
        label={props.label}
        checked={props.checked}
        disabled={props.disabled}
        onChange={props.onChange}
      />
    </div>
  );
}

function PolicyDialog(props: {
  policy: ChannelAccessPolicy;
  zh: boolean;
  onClose: () => void;
  onSaved: (policy: ChannelAccessPolicy) => void;
}) {
  const initial = untrack(() => props.policy);
  const [dmPolicy, setDmPolicy] = createSignal(initial.dmPolicy);
  const [allowlist, setAllowlist] = createSignal(initial.allowlistActorIds.join(", "));
  const [grant, setGrant] = createSignal(initial.grantCeiling);
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  const [confirming, setConfirming] = createSignal(false);
  async function save(confirmed = false) {
    if (permissionScopeRisk(grant().permissionPolicy).hasUnrestrictedScope && !confirmed) {
      setConfirming(true);
      return;
    }
    setBusy(true);
    setFailure(undefined);
    try {
      const policy = await commands.updateChannelAccessPolicy({
        accountId: props.policy.accountId,
        dmPolicy: dmPolicy(),
        allowlistActorIds: parseIdentifierList(allowlist()),
        grantCeiling: {
          ...grant(),
        },
        expectedRevision: props.policy.revision,
      });
      props.onSaved(policy);
      props.onClose();
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog
      open
      title={props.zh ? "消息策略" : "Messaging policy"}
      closeLabel={props.zh ? "关闭" : "Close"}
      loading={busy()}
      onOpenChange={(open) => !open && !busy() && props.onClose()}
    >
      <div class="integration-form integration-policy-dialog">
        <LabeledSelect
          label={props.zh ? "私聊策略" : "DM policy"}
          value={dmPolicy()}
          onChange={(value) => setDmPolicy(value as ChannelDmPolicy)}
          options={[
            { value: "pairing", label: props.zh ? "连接码" : "Pairing" },
            { value: "allowlist", label: props.zh ? "允许名单" : "Allowlist" },
            { value: "open", label: props.zh ? "开放" : "Open" },
            { value: "disabled", label: props.zh ? "禁用" : "Disabled" },
          ]}
        />
        <Show when={dmPolicy() === "allowlist"}>
          <TextField
            label={props.zh ? "允许的 Sender ID" : "Allowed sender IDs"}
            value={allowlist()}
            disabled={busy()}
            onInput={(event) => setAllowlist(event.currentTarget.value)}
          />
        </Show>
        <Show when={dmPolicy() === "open"}>
          <StatusBanner tone="warning">
            {props.zh ? "任何私聊发送者都可创建会话。" : "Any DM sender can create a session."}
          </StatusBanner>
        </Show>
        <ChannelGrantEditor value={grant()} disabled={busy()} zh={props.zh} onChange={setGrant} />
        <Show when={failure()}>
          {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
        </Show>
        <div class="integration-form-actions">
          <Button disabled={busy()} onClick={props.onClose}>
            {props.zh ? "取消" : "Cancel"}
          </Button>
          <Button variant="primary" disabled={busy()} onClick={() => void save()}>
            {props.zh ? "保存" : "Save"}
          </Button>
        </div>
        <PermissionScopeConfirmation
          open={confirming()}
          policy={grant().permissionPolicy}
          zh={props.zh}
          onCancel={() => setConfirming(false)}
          onConfirm={() => {
            setConfirming(false);
            void save(true);
          }}
        />
      </div>
    </Dialog>
  );
}

function PairingDialog(props: {
  account: IntegrationProviderAccount;
  provider: IntegrationProviderDefinition;
  zh: boolean;
  onClose: () => void;
}) {
  const [draft, setDraft] = createSignal<PairingDraft>({
    target: "dm_identity",
    groupHistoryPolicy: null,
    topicPolicy: null,
    mentionPolicy: null,
    grant: EMPTY_GRANT,
  });
  const [code, setCode] = createSignal<ChannelPairingCode>();
  const [failure, setFailure] = createSignal<string>();
  const [busy, setBusy] = createSignal(false);
  const [confirming, setConfirming] = createSignal(false);
  const supportsGroups = () => props.provider.capabilities.includes("group");
  const supportsTopics = () => props.provider.capabilities.includes("topic");
  async function generate(confirmed = false) {
    setBusy(true);
    setFailure(undefined);
    try {
      const value = draft();
      if (
        value.target === "group_conversation" &&
        (!value.groupHistoryPolicy ||
          !value.mentionPolicy ||
          (supportsTopics() && !value.topicPolicy))
      ) {
        setFailure(
          props.zh
            ? "请选择群历史、话题历史和 @ 策略。"
            : "Select group history, topic history, and mention policy.",
        );
        return;
      }
      if (permissionScopeRisk(value.grant.permissionPolicy).hasUnrestrictedScope && !confirmed) {
        setConfirming(true);
        return;
      }
      setCode(
        await commands.createChannelPairingCode({
          accountId: props.account.id,
          target: value.target,
          groupHistoryPolicy:
            value.target === "group_conversation" ? value.groupHistoryPolicy : null,
          topicPolicy:
            value.target === "group_conversation" && supportsTopics()
              ? value.topicPolicy!
              : "inherit_group",
          mentionPolicy: value.target === "group_conversation" ? value.mentionPolicy! : "disabled",
          grant: value.grant,
        }),
      );
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog
      open
      title={props.zh ? "创建连接码" : "Create pairing code"}
      closeLabel={props.zh ? "关闭" : "Close"}
      onOpenChange={(open) => !open && props.onClose()}
    >
      <div class="integration-form integration-pairing-dialog">
        <div
          class="integration-segmented"
          role="group"
          aria-label={props.zh ? "授权目标" : "Authorization target"}
        >
          <Button
            variant={draft().target === "dm_identity" ? "primary" : "default"}
            onClick={() => setDraft((value) => ({ ...value, target: "dm_identity" }))}
          >
            <Link size={14} /> {props.zh ? "私聊身份" : "DM identity"}
          </Button>
          <Show when={supportsGroups()}>
            <Button
              variant={draft().target === "group_conversation" ? "primary" : "default"}
              onClick={() => setDraft((value) => ({ ...value, target: "group_conversation" }))}
            >
              <Users size={14} /> {props.zh ? "群会话" : "Group"}
            </Button>
          </Show>
        </div>
        <Show when={draft().target === "group_conversation"}>
          <LabeledSelect
            label={props.zh ? "群历史" : "Group history"}
            value={draft().groupHistoryPolicy ?? ""}
            onChange={(groupHistoryPolicy) =>
              setDraft((value) => ({
                ...value,
                groupHistoryPolicy: groupHistoryPolicy
                  ? (groupHistoryPolicy as ChannelGroupHistoryPolicy)
                  : null,
              }))
            }
            options={[
              { value: "", label: props.zh ? "请选择" : "Select" },
              { value: "shared", label: props.zh ? "共享" : "Shared" },
              { value: "per_sender", label: props.zh ? "成员私有" : "Per sender" },
            ]}
          />
          <Show when={supportsTopics()}>
            <LabeledSelect
              label={props.zh ? "话题历史" : "Topic history"}
              value={draft().topicPolicy ?? ""}
              onChange={(topicPolicy) =>
                setDraft((value) => ({
                  ...value,
                  topicPolicy: topicPolicy ? (topicPolicy as ChannelTopicPolicy) : null,
                }))
              }
              options={[
                { value: "", label: props.zh ? "请选择" : "Select" },
                { value: "inherit_group", label: props.zh ? "继承群历史" : "Inherit group" },
                { value: "isolate_topic", label: props.zh ? "隔离话题" : "Isolate topic" },
              ]}
            />
          </Show>
          <LabeledSelect
            label={props.zh ? "@ 策略" : "Mention policy"}
            value={draft().mentionPolicy ?? ""}
            onChange={(mentionPolicy) =>
              setDraft((value) => ({
                ...value,
                mentionPolicy: mentionPolicy ? (mentionPolicy as ChannelMentionPolicy) : null,
              }))
            }
            options={[
              { value: "", label: props.zh ? "请选择" : "Select" },
              { value: "required", label: props.zh ? "必须 @" : "Required" },
              { value: "all_messages", label: props.zh ? "全部消息" : "All messages" },
              { value: "disabled", label: props.zh ? "禁用" : "Disabled" },
            ]}
          />
        </Show>
        <ChannelGrantEditor
          value={draft().grant}
          disabled={busy()}
          zh={props.zh}
          onChange={(grant) => setDraft((value) => ({ ...value, grant }))}
        />
        <Show when={code()}>
          {(value) => (
            <div class="integration-pairing-code">
              <code>{value().code}</code>
              <span>{new Date(value().expiresAtMs).toLocaleTimeString()}</span>
            </div>
          )}
        </Show>
        <Show when={failure()}>
          {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
        </Show>
        <div class="integration-form-actions">
          <Button onClick={props.onClose}>{props.zh ? "取消" : "Cancel"}</Button>
          <Button variant="primary" disabled={busy()} onClick={() => void generate()}>
            <KeyRound size={15} /> {props.zh ? "生成" : "Generate"}
          </Button>
        </div>
        <PermissionScopeConfirmation
          open={confirming()}
          policy={draft().grant.permissionPolicy}
          zh={props.zh}
          onCancel={() => setConfirming(false)}
          onConfirm={() => {
            setConfirming(false);
            void generate(true);
          }}
        />
      </div>
    </Dialog>
  );
}

function parseIdentifierList(value: string): string[] {
  return [
    ...new Set(
      value
        .split(/[\s,]+/)
        .map((item) => item.trim())
        .filter(Boolean),
    ),
  ];
}

function LabeledSelect(props: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
}) {
  return (
    <NativeSelect
      label={props.label}
      value={props.value}
      onChange={(event) => props.onChange(event.currentTarget.value)}
    >
      <For each={props.options}>
        {(option) => <option value={option.value}>{option.label}</option>}
      </For>
    </NativeSelect>
  );
}

function AuthorizationList(props: {
  values: ChannelAuthorization[];
  zh: boolean;
  failure: string | undefined;
  onLink: (authorization: ChannelAuthorization) => void;
  onAdd: () => void;
  onEdit: (authorization: ChannelAuthorization) => void;
}) {
  return (
    <div class="integration-form integration-authorization-list">
      <div class="integration-routes-heading">
        <strong>{props.zh ? "精确授权" : "Exact authorizations"}</strong>
        <Button onClick={props.onAdd}>
          <Plus size={14} /> {props.zh ? "添加" : "Add"}
        </Button>
      </div>
      <Show
        when={props.values.length > 0}
        fallback={
          <span class="integration-route-empty">{props.zh ? "暂无授权" : "No authorizations"}</span>
        }
      >
        <For each={props.values}>
          {(authorization) => (
            <div class="integration-authorization-row">
              <span>
                <strong>{authorization.address.chatId}</strong>
                <small>
                  {authorization.target === "dm_identity"
                    ? props.zh
                      ? "私聊"
                      : "DM"
                    : authorization.groupHistoryPolicy === "shared"
                      ? props.zh
                        ? "群共享"
                        : "Shared group"
                      : props.zh
                        ? "成员私有"
                        : "Per sender"}
                </small>
              </span>
              <div class="integration-account-actions">
                <Button onClick={() => props.onEdit(authorization)}>
                  {props.zh ? "编辑" : "Edit"}
                </Button>
                <Show when={authorization.target === "dm_identity" && authorization.enabled}>
                  <Button onClick={() => props.onLink(authorization)}>
                    <Link size={14} /> {props.zh ? "关联身份" : "Link identity"}
                  </Button>
                </Show>
                <Badge tone={authorization.enabled ? "success" : "neutral"}>
                  {authorization.enabled
                    ? props.zh
                      ? "已启用"
                      : "Enabled"
                    : props.zh
                      ? "已停用"
                      : "Disabled"}
                </Badge>
              </div>
            </div>
          )}
        </For>
      </Show>
      <Show when={props.failure}>
        {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
      </Show>
    </div>
  );
}

function IlinkQrDialog(props: {
  session: IlinkQrSession;
  zh: boolean;
  failure: string | undefined;
  onPoll: () => void;
  onRefresh: () => void | Promise<void>;
  onCancel: () => void;
}) {
  const [clock, setClock] = createSignal(Date.now());
  const remaining = () => Math.max(0, Math.ceil((props.session.expiresAtMs - clock()) / 1000));
  onMount(() => {
    let refreshing = false;
    const clockTimer = window.setInterval(() => setClock(Date.now()), 1_000);
    const pollTimer = window.setInterval(() => untrack(() => props.onPoll()), 2_500);
    const refreshTimer = window.setInterval(
      () =>
        untrack(() => {
          if (
            remaining() === 0 &&
            !refreshing &&
            !["confirmed", "cancelled"].includes(props.session.state)
          ) {
            refreshing = true;
            Promise.resolve(props.onRefresh()).finally(() => {
              refreshing = false;
            });
          }
        }),
      1_000,
    );
    onCleanup(() => {
      window.clearInterval(clockTimer);
      window.clearInterval(pollTimer);
      window.clearInterval(refreshTimer);
    });
  });
  return (
    <Dialog
      open
      title={props.zh ? "连接微信 iLink" : "Connect WeChat iLink"}
      closeLabel={props.zh ? "关闭" : "Close"}
      onOpenChange={(open) => !open && props.onCancel()}
    >
      <div class="integration-form integration-ilink-qr">
        <Show when={props.session.qrContent}>
          <img
            src={props.session.qrContent}
            alt={props.zh ? "微信 iLink 二维码" : "WeChat iLink QR code"}
          />
        </Show>
        <div class="integration-pairing-code">
          <strong>{ilinkStateLabel(props.session.state, props.zh)}</strong>
          <span>{remaining()}s</span>
        </div>
        <Show when={props.failure}>
          {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
        </Show>
        <div class="integration-form-actions">
          <Button onClick={props.onCancel}>{props.zh ? "取消" : "Cancel"}</Button>
          <Button
            disabled={props.session.state !== "expired" && remaining() > 0}
            onClick={props.onRefresh}
          >
            <RefreshCw size={15} /> {props.zh ? "刷新" : "Refresh"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

function ilinkStateLabel(state: string, zh: boolean) {
  const labels: Record<string, [string, string]> = {
    waiting: ["等待扫码", "Waiting for scan"],
    scanned: ["已扫码，等待确认", "Scanned, awaiting confirmation"],
    expired: ["二维码已过期", "QR code expired"],
    confirmed: ["已连接", "Connected"],
    cancelled: ["已取消", "Cancelled"],
  };
  return (labels[state] ?? [state, state])[zh ? 0 : 1];
}

function validateDraft(provider: IntegrationProviderDefinition, draft: AccountDraft, zh: boolean) {
  if (!draft.displayName.trim()) return zh ? "请填写账户名称" : "Account name is required";
  if (!draft.apiAccess && !draft.messaging)
    return zh ? "请至少启用企业 API 或消息 Channel" : "Enable Enterprise API or Messaging Channel";
  if (
    draft.mode !== "capabilities" &&
    provider.credentialFields.some(
      (field) =>
        credentialFieldEnabled(field.capability, draft.apiAccess, draft.messaging) &&
        field.required &&
        !(draft.fields[field.id] ?? "").trim(),
    )
  )
    return zh ? "请填写平台要求的凭据" : "Required credentials are missing";
  return undefined;
}

function credentialFieldEnabled(
  capability: IntegrationProviderDefinition["credentialFields"][number]["capability"],
  apiAccess: boolean,
  messaging: boolean,
) {
  if (!capability) return true;
  if (capability === "api_access") return apiAccess;
  if (capability === "messaging" || capability === "proactive_delivery") return messaging;
  return true;
}

function credential(
  provider: IntegrationProviderId,
  fields: Record<string, string>,
): IntegrationCredentialInput {
  const value = (id: string) => fields[id] ?? "";
  switch (provider) {
    case "dingtalk":
      return {
        providerId: provider,
        clientId: value("clientId").trim(),
        clientSecret: value("clientSecret"),
        agentId: value("agentId").trim() || null,
        robotCode: value("robotCode").trim() || null,
      };
    case "feishu":
      return { providerId: provider, appId: value("appId").trim(), appSecret: value("appSecret") };
    case "wecom_ai_bot":
      return { providerId: provider, botId: value("botId").trim(), secret: value("secret") };
    case "wecom_app":
      return {
        providerId: provider,
        corpId: value("corpId").trim(),
        corpSecret: value("corpSecret"),
        agentId: value("agentId").trim(),
        callbackToken: value("callbackToken"),
        encodingAesKey: value("encodingAesKey"),
        externalHttpsUrl: value("externalHttpsUrl").trim(),
      };
    case "wechat_ilink":
      return {
        providerId: provider,
        botToken: value("botToken"),
        botId: value("botId").trim(),
        baseUrl: value("baseUrl").trim(),
      };
  }
}

function dialogTitle(
  mode: AccountDialogMode,
  provider: IntegrationProviderDefinition,
  zh: boolean,
) {
  const name = zh ? provider.nameZh : provider.nameEn;
  if (mode === "capabilities") return zh ? `${name} 设置` : `${name} settings`;
  if (mode === "credentials") return zh ? `${name} 凭据` : `${name} credentials`;
  return zh ? `连接 ${name}` : `Connect ${name}`;
}

function capabilitySummary(account: IntegrationProviderAccount, zh: boolean) {
  const values = [
    account.apiAccessEnabled && (zh ? "企业 API" : "Enterprise API"),
    account.messagingEnabled && (zh ? "消息 Channel" : "Messaging Channel"),
  ].filter(Boolean);
  return values.join(" · ");
}

function stateLabel(state: IntegrationProviderAccount["state"], zh: boolean) {
  const labels: Record<IntegrationProviderAccount["state"], [string, string]> = {
    draft: ["草稿", "Draft"],
    awaiting_auth: ["等待认证", "Awaiting auth"],
    starting: ["启动中", "Starting"],
    healthy: ["正常", "Healthy"],
    degraded: ["性能下降", "Degraded"],
    needs_attention: ["需要处理", "Needs attention"],
    revoked: ["已撤销", "Revoked"],
    removing: ["移除中", "Removing"],
  };
  return labels[state][zh ? 0 : 1];
}

function transportLabel(transport: IntegrationProviderDefinition["transport"], zh: boolean) {
  const labels = {
    encrypted_callback: ["加密回调", "Encrypted callback"],
    stream: ["Stream 长连接", "Stream connection"],
    long_connection: ["长连接", "Long connection"],
    web_socket: ["WebSocket", "WebSocket"],
    qr_long_poll: ["扫码长轮询", "QR long poll"],
  } as const;
  return labels[transport][zh ? 0 : 1];
}

function formatTime(value: number | null, zh: boolean) {
  return value ? new Date(value).toLocaleString() : zh ? "暂无" : "None";
}

function providerRuntimeError(code: string, zh: boolean) {
  const messages: Record<string, [string, string]> = {
    provider_transport_unavailable: [
      "消息连接暂时中断，Hachimi 将自动重连",
      "The messaging connection was interrupted; Hachimi will reconnect automatically",
    ],
    provider_authentication_expired: [
      "平台授权已过期，请更新账户凭据",
      "Platform authorization expired; update the account credentials",
    ],
    provider_credentials_or_transport_require_attention: [
      "平台凭据或连接配置需要检查",
      "The platform credentials or connection settings require attention",
    ],
    channel_sidecar_health_rejected: [
      "平台消息组件未通过健康检查，可稍后重试",
      "The messaging component failed its health check; retry shortly",
    ],
    channel_sidecar_unavailable: [
      "平台消息组件暂时不可用，Hachimi 将继续恢复",
      "The messaging component is unavailable; Hachimi will keep recovering",
    ],
  };
  const message =
    messages[code]?.[zh ? 0 : 1] ??
    (zh ? "平台消息连接异常" : "Platform messaging connection failed");
  return `${message} (${code})`;
}
