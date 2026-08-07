import { commandFailure, commands, type AgentPermissionPolicy } from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Button, SettingsCard, SettingsSection, StatusBanner } from "@hachimi/ui";
import { Show, createSignal, onMount } from "solid-js";

import { PermissionPolicyEditor, createPermissionPolicy } from "./permission-policy-editor";
import { PermissionScopeConfirmation } from "./permission-scope-confirmation";
import { permissionScopeRisk } from "./permission-scope-risk";

/** The Pet policy is a persistent Agent profile, so it belongs to Workbench settings. */
export function PetPermissionSettings() {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const [draft, setDraft] = createSignal<AgentPermissionPolicy>(createPermissionPolicy());
  const [loading, setLoading] = createSignal(true);
  const [saving, setSaving] = createSignal(false);
  const [dirty, setDirty] = createSignal(false);
  const [notice, setNotice] = createSignal<{ tone: "success" | "danger"; text: string }>();
  const [confirming, setConfirming] = createSignal(false);

  async function load() {
    setLoading(true);
    try {
      const config = await commands.getSessionPermissionConfig({
        sessionId: null,
        entryProfile: "pet_conversation",
      });
      setDraft(config.policy);
      setDirty(false);
      setNotice(undefined);
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setLoading(false);
    }
  }

  async function save(confirmed = false) {
    const risk = permissionScopeRisk(draft());
    if (risk.hasUnrestrictedScope && !confirmed) {
      setConfirming(true);
      return;
    }
    setSaving(true);
    setNotice(undefined);
    try {
      const config = await commands.updateSessionPermissionConfig({
        sessionId: null,
        entryProfile: "pet_conversation",
        expectedRevision: draft().revision,
        config: { extraAuthorizations: [], policy: draft() },
      });
      setDraft(config.policy);
      setDirty(false);
      setNotice({
        tone: "success",
        text: zh() ? "Pet Agent 权限已保存。" : "Pet Agent permissions saved.",
      });
    } catch (error) {
      setNotice({ tone: "danger", text: commandFailure(error).message });
    } finally {
      setSaving(false);
    }
  }

  function update(next: AgentPermissionPolicy) {
    if (next.level === "read_only") {
      next.rules.connectors = next.rules.connectors.map((rule) => ({
        ...rule,
        readOnlyActions: [...rule.actions],
      }));
    }
    setDraft(next);
    setDirty(true);
  }

  onMount(() => void load());

  return (
    <SettingsSection title={zh() ? "Pet Agent 权限" : "Pet Agent permissions"}>
      <Show when={notice()}>
        {(value) => <StatusBanner tone={value().tone}>{value().text}</StatusBanner>}
      </Show>
      <SettingsCard>
        <PermissionPolicyEditor
          value={draft()}
          testId="pet-permission"
          zh={zh()}
          disabled={loading() || saving()}
          onChange={update}
        />
        <div class="settings-card-actions">
          <Button
            variant="primary"
            data-testid="pet-permission-save"
            disabled={loading() || saving() || !dirty()}
            loading={saving()}
            onClick={() => void save()}
          >
            {zh() ? "保存权限" : "Save permissions"}
          </Button>
        </div>
      </SettingsCard>
      <PermissionScopeConfirmation
        open={confirming()}
        policy={draft()}
        zh={zh()}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          void save(true);
        }}
      />
    </SettingsSection>
  );
}
