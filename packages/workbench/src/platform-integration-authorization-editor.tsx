import {
  commandFailure,
  commands,
  type ChannelAuthorization,
  type ChannelAuthorizationTarget,
  type ChannelGrant,
  type ChannelGroupHistoryPolicy,
  type ChannelMentionPolicy,
  type ChannelTopicPolicy,
  type IntegrationProviderAccount,
  type IntegrationProviderDefinition,
} from "@hachimi/contracts";
import { Button, Dialog, NativeSelect, StatusBanner, TextField } from "@hachimi/ui";
import { Show, createSignal, untrack } from "solid-js";

import { ChannelGrantEditor } from "./platform-integration-grant-editor";
import { createPermissionPolicy } from "./permission-policy-editor";
import { PermissionScopeConfirmation } from "./permission-scope-confirmation";
import { permissionScopeRisk } from "./permission-scope-risk";

const EMPTY_GRANT: ChannelGrant = {
  permissionPolicy: createPermissionPolicy(),
  skillIds: [],
  mcpServerIds: [],
  connectorSelections: [],
  readOnlyWorkspaceRoots: [],
  networkHosts: [],
};

export function AuthorizationEditor(props: {
  account: IntegrationProviderAccount;
  provider: IntegrationProviderDefinition;
  value?: ChannelAuthorization;
  zh: boolean;
  onClose: () => void;
  onSaved: (value: ChannelAuthorization) => void;
}) {
  const initial = untrack(() => props.value);
  const [target, setTarget] = createSignal<ChannelAuthorizationTarget>(
    initial?.target ?? "dm_identity",
  );
  const [chatId, setChatId] = createSignal(initial?.address.chatId ?? "");
  const [actorId, setActorId] = createSignal(initial?.actorId ?? "");
  const [topicId, setTopicId] = createSignal(initial?.address.topicId ?? "");
  const [history, setHistory] = createSignal<ChannelGroupHistoryPolicy>(
    initial?.groupHistoryPolicy ?? "shared",
  );
  const [topicPolicy, setTopicPolicy] = createSignal<ChannelTopicPolicy>(
    initial?.topicPolicy ?? "inherit_group",
  );
  const [mentionPolicy, setMentionPolicy] = createSignal<ChannelMentionPolicy>(
    initial?.mentionPolicy ?? "required",
  );
  const [grant, setGrant] = createSignal(initial?.grant ?? EMPTY_GRANT);
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  const [confirming, setConfirming] = createSignal(false);
  const supportsGroup = () => props.provider.capabilities.includes("group");
  const supportsTopic = () => props.provider.capabilities.includes("topic");

  async function save(enabled = true, confirmed = false) {
    if (!chatId().trim() || (target() === "dm_identity" && !actorId().trim())) {
      setFailure(
        props.zh
          ? "请填写稳定会话 ID 和发送者 ID。"
          : "Stable conversation and sender IDs are required.",
      );
      return;
    }
    if (
      enabled &&
      permissionScopeRisk(grant().permissionPolicy).hasUnrestrictedScope &&
      !confirmed
    ) {
      setConfirming(true);
      return;
    }
    setBusy(true);
    setFailure(undefined);
    try {
      const value = await commands.upsertChannelAuthorization({
        id: props.value?.id ?? crypto.randomUUID(),
        accountId: props.account.id,
        target: target(),
        address: {
          providerId: props.account.providerId,
          accountId: props.account.id,
          tenantKey: "",
          chatKind: target() === "dm_identity" ? "dm" : "group",
          chatId: chatId().trim(),
          topicId:
            target() === "group_conversation" && supportsTopic() && topicId().trim()
              ? topicId().trim()
              : null,
        },
        actorId: target() === "dm_identity" ? actorId().trim() : null,
        groupHistoryPolicy: target() === "group_conversation" ? history() : null,
        topicPolicy: target() === "group_conversation" ? topicPolicy() : "inherit_group",
        mentionPolicy: target() === "group_conversation" ? mentionPolicy() : "disabled",
        grant: grant(),
        enabled,
        expectedRevision: props.value?.revision ?? null,
      });
      props.onSaved(value);
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
      size="wide"
      title={
        props.value
          ? props.zh
            ? "编辑会话授权"
            : "Edit conversation access"
          : props.zh
            ? "添加会话授权"
            : "Add conversation access"
      }
      closeLabel={props.zh ? "关闭" : "Close"}
      loading={busy()}
      onOpenChange={(open) => !open && !busy() && props.onClose()}
    >
      <div class="integration-form integration-authorization-editor">
        <NativeSelect
          label={props.zh ? "授权目标" : "Authorization target"}
          value={target()}
          disabled={busy()}
          onChange={(event) => setTarget(event.currentTarget.value as ChannelAuthorizationTarget)}
        >
          <option value="dm_identity">{props.zh ? "私聊身份" : "DM identity"}</option>
          <Show when={supportsGroup()}>
            <option value="group_conversation">{props.zh ? "群会话" : "Group conversation"}</option>
          </Show>
        </NativeSelect>
        <TextField
          label={
            target() === "dm_identity"
              ? props.zh
                ? "私聊会话 ID"
                : "DM conversation ID"
              : props.zh
                ? "群会话 ID"
                : "Group conversation ID"
          }
          value={chatId()}
          disabled={busy()}
          onInput={(event) => setChatId(event.currentTarget.value)}
        />
        <Show when={target() === "dm_identity"}>
          <TextField
            label={props.zh ? "发送者 ID" : "Sender ID"}
            value={actorId()}
            disabled={busy()}
            onInput={(event) => setActorId(event.currentTarget.value)}
          />
        </Show>
        <Show when={target() === "group_conversation"}>
          <div class="integration-credential-grid">
            <NativeSelect
              label={props.zh ? "群历史" : "Group history"}
              value={history()}
              disabled={busy()}
              onChange={(event) =>
                setHistory(event.currentTarget.value as ChannelGroupHistoryPolicy)
              }
            >
              <option value="shared">{props.zh ? "共享" : "Shared"}</option>
              <option value="per_sender">{props.zh ? "成员私有" : "Per sender"}</option>
            </NativeSelect>
            <NativeSelect
              label={props.zh ? "@ 策略" : "Mention policy"}
              value={mentionPolicy()}
              disabled={busy()}
              onChange={(event) =>
                setMentionPolicy(event.currentTarget.value as ChannelMentionPolicy)
              }
            >
              <option value="required">{props.zh ? "必须 @" : "Required"}</option>
              <option value="all_messages">{props.zh ? "全部消息" : "All messages"}</option>
              <option value="disabled">{props.zh ? "禁用" : "Disabled"}</option>
            </NativeSelect>
          </div>
          <Show when={supportsTopic()}>
            <div class="integration-credential-grid">
              <NativeSelect
                label={props.zh ? "话题历史" : "Topic history"}
                value={topicPolicy()}
                disabled={busy()}
                onChange={(event) =>
                  setTopicPolicy(event.currentTarget.value as ChannelTopicPolicy)
                }
              >
                <option value="inherit_group">{props.zh ? "继承群历史" : "Inherit group"}</option>
                <option value="isolate_topic">{props.zh ? "隔离话题" : "Isolate topic"}</option>
              </NativeSelect>
              <Show when={topicPolicy() === "isolate_topic"}>
                <TextField
                  label={props.zh ? "话题 ID" : "Topic ID"}
                  value={topicId()}
                  disabled={busy()}
                  onInput={(event) => setTopicId(event.currentTarget.value)}
                />
              </Show>
            </div>
          </Show>
        </Show>
        <ChannelGrantEditor value={grant()} disabled={busy()} zh={props.zh} onChange={setGrant} />
        <Show when={failure()}>
          {(message) => <StatusBanner tone="danger">{message()}</StatusBanner>}
        </Show>
        <div class="integration-form-actions">
          <Show when={props.value?.enabled}>
            <Button variant="danger" disabled={busy()} onClick={() => void save(false)}>
              {props.zh ? "停用" : "Disable"}
            </Button>
          </Show>
          <Button disabled={busy()} onClick={props.onClose}>
            {props.zh ? "取消" : "Cancel"}
          </Button>
          <Button variant="primary" disabled={busy()} onClick={() => void save(true)}>
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
            void save(true, true);
          }}
        />
      </div>
    </Dialog>
  );
}
