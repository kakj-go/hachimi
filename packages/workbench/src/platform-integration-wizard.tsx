import {
  commands,
  type IntegrationCredentialInput,
  type IntegrationProviderAccount,
  type IntegrationProviderDefinition,
  type IlinkQrSession,
} from "@hachimi/contracts";
import { Button, Dialog, RefreshCw, StatusBanner, Switch as Toggle, TextField } from "@hachimi/ui";
import { For, Show, createMemo, createSignal, onCleanup, onMount, untrack } from "solid-js";

import { integrationFailureMessage } from "./platform-integration-errors";

export function IntegrationAccountWizard(props: {
  provider: IntegrationProviderDefinition;
  zh: boolean;
  onClose: () => void;
  onCompleted: (account: IntegrationProviderAccount, probeFailed: boolean) => void;
}) {
  const [accountId] = createSignal(crypto.randomUUID());
  const [displayName, setDisplayName] = createSignal(
    untrack(() => (props.zh ? `${props.provider.nameZh}账户` : `${props.provider.nameEn} account`)),
  );
  const [fields, setFields] = createSignal<Record<string, string>>(
    untrack(() =>
      Object.fromEntries(props.provider.credentialFields.map((field) => [field.id, ""])),
    ),
  );
  const [apiAccess, setApiAccess] = createSignal(
    untrack(() => props.provider.capabilities.includes("api_access")),
  );
  const [messaging, setMessaging] = createSignal(true);
  const [qr, setQr] = createSignal<IlinkQrSession>();
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  const activeCredentialFields = createMemo(() =>
    props.provider.credentialFields.filter((field) =>
      credentialFieldEnabled(field.capability, apiAccess(), messaging()),
    ),
  );

  async function connect() {
    setFailure(undefined);
    if (!displayName().trim()) {
      setFailure(props.zh ? "请填写账户名称。" : "Account name is required.");
      return;
    }
    if (!apiAccess() && !messaging()) {
      setFailure(props.zh ? "请至少启用一项能力。" : "Enable at least one capability.");
      return;
    }
    if (
      activeCredentialFields().some((field) => field.required && !(fields()[field.id] ?? "").trim())
    ) {
      setFailure(props.zh ? "请填写平台要求的凭据。" : "Required credentials are missing.");
      return;
    }

    setBusy(true);
    try {
      if (props.provider.id === "wechat_ilink") {
        setQr(
          await commands.beginIlinkQrLogin({
            accountId: accountId(),
            displayName: displayName().trim(),
          }),
        );
        return;
      }
      const created = await commands.upsertEnterpriseIntegration({
        id: accountId(),
        displayName: displayName().trim(),
        credential: credential(props.provider.id, fields()),
        apiAccessEnabled: apiAccess(),
        messagingEnabled: messaging(),
        expectedConfigRevision: null,
      });
      try {
        const result = await commands.probeEnterpriseIntegration(created.id);
        props.onCompleted(result.account, false);
      } catch {
        props.onCompleted(created, true);
      }
    } catch (error) {
      setFailure(integrationFailureMessage(error, props.zh));
    } finally {
      setBusy(false);
    }
  }

  async function pollQr() {
    const current = qr();
    if (!current) return;
    try {
      const value = await commands.pollIlinkQrLogin(current.accountId);
      setQr(value);
      if (value.state !== "confirmed") return;
      const created = (await commands.listEnterpriseIntegrations()).find(
        (candidate) => candidate.id === value.accountId,
      );
      if (!created) throw new Error("Confirmed iLink account was not persisted");
      try {
        const result = await commands.probeEnterpriseIntegration(created.id);
        props.onCompleted(result.account, false);
      } catch {
        props.onCompleted(created, true);
      }
    } catch (error) {
      setFailure(integrationFailureMessage(error, props.zh));
    }
  }

  async function refreshQr() {
    try {
      setQr(
        await commands.beginIlinkQrLogin({
          accountId: accountId(),
          displayName: displayName().trim(),
        }),
      );
    } catch (error) {
      setFailure(integrationFailureMessage(error, props.zh));
    }
  }

  async function close() {
    if (qr()) await commands.cancelIlinkQrLogin(accountId()).catch(() => undefined);
    props.onClose();
  }

  return (
    <Dialog
      open
      size="wide"
      title={props.zh ? `连接 ${props.provider.nameZh}` : `Connect ${props.provider.nameEn}`}
      description={
        props.zh
          ? "填写账户与凭据后一次完成创建，随后自动检测连接状态。"
          : "Enter the account and credentials once; connection checks run automatically."
      }
      closeLabel={props.zh ? "关闭" : "Close"}
      loading={busy()}
      onOpenChange={(open) => !open && !busy() && void close()}
    >
      <div class="integration-form integration-connect-form">
        <Show
          when={!qr()}
          fallback={
            <WizardQr session={qr()!} zh={props.zh} onPoll={pollQr} onRefresh={refreshQr} />
          }
        >
          <TextField
            label={props.zh ? "账户名称" : "Account name"}
            value={displayName()}
            disabled={busy()}
            onInput={(event) => setDisplayName(event.currentTarget.value)}
          />
          <div class="integration-capability-switches">
            <Show when={props.provider.capabilities.includes("api_access")}>
              <CapabilityToggle
                label={props.zh ? "企业 API" : "Enterprise API"}
                description={
                  props.zh ? "允许 Agent 调用该平台的企业接口。" : "Allow enterprise API calls."
                }
                checked={apiAccess()}
                onChange={setApiAccess}
              />
            </Show>
            <CapabilityToggle
              label={props.zh ? "消息 Channel" : "Messaging Channel"}
              description={
                props.zh ? "接收消息并投递 Agent 回复。" : "Receive messages and deliver replies."
              }
              checked={messaging()}
              onChange={setMessaging}
            />
          </div>
          <Show when={props.provider.id !== "wechat_ilink"}>
            <div class="integration-credential-grid">
              <For each={activeCredentialFields()}>
                {(field) => (
                  <TextField
                    label={field.label}
                    value={fields()[field.id] ?? ""}
                    type={
                      field.kind === "secret"
                        ? "password"
                        : field.kind === "integer"
                          ? "number"
                          : "text"
                    }
                    disabled={busy()}
                    onInput={(event) =>
                      setFields((current) => ({
                        ...current,
                        [field.id]: event.currentTarget.value,
                      }))
                    }
                  />
                )}
              </For>
            </div>
            <Show when={props.provider.id === "wecom_app" && messaging()}>
              <div class="integration-callback-path">
                /v1/channels/wecom_app/{accountId()}/callback
              </div>
            </Show>
          </Show>
          <StatusBanner tone="neutral">
            {props.zh
              ? "消息策略默认使用连接码，Agent 权限默认最小授权；创建后可在账户卡片中修改。"
              : "Messaging defaults to pairing and Agent access defaults to least privilege; edit either from the account card after creation."}
          </StatusBanner>
        </Show>
        <Show when={props.provider.sourceStatus === "public_qualification_unverified"}>
          <StatusBanner tone="warning">
            {props.zh
              ? "公开资格与条款尚未独立核实"
              : "Public eligibility and terms are not independently verified"}
          </StatusBanner>
        </Show>
        <Show when={failure()}>
          {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
        </Show>
        <div class="integration-form-actions">
          <Button disabled={busy()} onClick={() => void close()}>
            {props.zh ? "取消" : "Cancel"}
          </Button>
          <Show when={!qr()}>
            <Button
              variant="primary"
              data-testid="integration-wizard-primary-action"
              disabled={busy()}
              onClick={() => void connect()}
            >
              {props.provider.id === "wechat_ilink"
                ? props.zh
                  ? "生成二维码"
                  : "Generate QR code"
                : props.zh
                  ? "连接并检测"
                  : "Connect and check"}
            </Button>
          </Show>
        </div>
      </div>
    </Dialog>
  );
}

function CapabilityToggle(props: {
  label: string;
  description: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div class="integration-capability-option">
      <span>
        <strong>{props.label}</strong>
        <small>{props.description}</small>
      </span>
      <Toggle label={props.label} checked={props.checked} onChange={props.onChange} />
    </div>
  );
}

function WizardQr(props: {
  session: IlinkQrSession;
  zh: boolean;
  onPoll: () => void;
  onRefresh: () => void | Promise<void>;
}) {
  const [clock, setClock] = createSignal(Date.now());
  const remaining = () => Math.max(0, Math.ceil((props.session.expiresAtMs - clock()) / 1_000));
  onMount(() => {
    const clockTimer = window.setInterval(() => setClock(Date.now()), 1_000);
    const pollTimer = window.setInterval(props.onPoll, 2_500);
    onCleanup(() => {
      window.clearInterval(clockTimer);
      window.clearInterval(pollTimer);
    });
  });
  return (
    <div class="integration-ilink-qr integration-wizard-qr">
      <Show when={props.session.qrContent}>
        <img
          src={props.session.qrContent}
          alt={props.zh ? "微信 iLink 二维码" : "WeChat iLink QR code"}
        />
      </Show>
      <div class="integration-pairing-code">
        <strong>{props.zh ? "等待扫码确认" : "Waiting for confirmation"}</strong>
        <span>{remaining()}s</span>
      </div>
      <Button disabled={remaining() > 0} onClick={props.onRefresh}>
        <RefreshCw size={15} /> {props.zh ? "刷新" : "Refresh"}
      </Button>
    </div>
  );
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
  provider: IntegrationProviderDefinition["id"],
  fields: Record<string, string>,
): IntegrationCredentialInput {
  const value = (id: string) => fields[id] ?? "";
  if (provider === "dingtalk")
    return {
      providerId: provider,
      clientId: value("clientId").trim(),
      clientSecret: value("clientSecret"),
      agentId: value("agentId").trim() || null,
      robotCode: value("robotCode").trim() || null,
    };
  if (provider === "feishu")
    return { providerId: provider, appId: value("appId").trim(), appSecret: value("appSecret") };
  if (provider === "wecom_ai_bot")
    return { providerId: provider, botId: value("botId").trim(), secret: value("secret") };
  if (provider === "wecom_app")
    return {
      providerId: provider,
      corpId: value("corpId").trim(),
      corpSecret: value("corpSecret"),
      agentId: value("agentId").trim(),
      callbackToken: value("callbackToken"),
      encodingAesKey: value("encodingAesKey"),
      externalHttpsUrl: value("externalHttpsUrl").trim(),
    };
  return { providerId: provider, botToken: "", botId: "", baseUrl: "" };
}
