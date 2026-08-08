import {
  commandFailure,
  commands,
  type AgentPermissionPolicy,
  type SkillRecord,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Button, History, SettingsSection, ShieldCheck, Sparkles, StatusBanner } from "@hachimi/ui";
import { Show, createSignal, onMount } from "solid-js";

import { AuthorizationWorkspace } from "./authorization-workspace";
import { PermissionPolicyEditor, createPermissionPolicy } from "./permission-policy-editor";
import { PermissionScopeConfirmation } from "./permission-scope-confirmation";
import { permissionScopeRisk } from "./permission-scope-risk";
import { skillDisplayName } from "./skill-display";
import { SkillPermissionEditor } from "./skill-permission-editor";

type PetPermissionSection = "permissions" | "skills" | "review";

/** The Pet policy is a persistent Agent profile, so it belongs to Workbench settings. */
export function PetPermissionSettings() {
  const i18n = useI18n();
  const zh = () => i18n.locale() === "zh-CN";
  const [section, setSection] = createSignal<PetPermissionSection>("permissions");
  const [draft, setDraft] = createSignal<AgentPermissionPolicy>(createPermissionPolicy());
  const [skills, setSkills] = createSignal<SkillRecord[]>([]);
  const [selectedSkillIds, setSelectedSkillIds] = createSignal<string[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [saving, setSaving] = createSignal(false);
  const [dirty, setDirty] = createSignal(false);
  const [notice, setNotice] = createSignal<{ tone: "success" | "danger"; text: string }>();
  const [confirming, setConfirming] = createSignal(false);

  async function load() {
    setLoading(true);
    try {
      const [config, nextSkills] = await Promise.all([
        commands.getSessionPermissionConfig({
          sessionId: null,
          entryProfile: "pet_conversation",
        }),
        commands.listSkills(),
      ]);
      setDraft(config.policy);
      setSelectedSkillIds(config.skillIds ?? []);
      setSkills(nextSkills);
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
        config: {
          extraAuthorizations: [],
          policy: draft(),
          skillIds: selectedSkillIds(),
        },
      });
      setDraft(config.policy);
      setSelectedSkillIds(config.skillIds ?? []);
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

  function updateSkills(ids: string[]) {
    setSelectedSkillIds(ids);
    setDirty(true);
  }

  onMount(() => void load());

  return (
    <SettingsSection title={zh() ? "Pet Agent 权限" : "Pet Agent permissions"}>
      <Show when={notice()}>
        {(value) => <StatusBanner tone={value().tone}>{value().text}</StatusBanner>}
      </Show>
      <AuthorizationWorkspace<PetPermissionSection>
        class="pet-authorization-workspace"
        ariaLabel={zh() ? "Pet Agent 配置" : "Pet Agent configuration"}
        sections={[
          {
            value: "permissions",
            label: zh() ? "Agent 权限" : "Agent permissions",
            description: zh()
              ? "设置 Pet Agent 可访问的文件、进程、网络、浏览器和桌面资源。"
              : "Set the files, processes, network, browser, and desktop resources Pet Agent can access.",
            icon: <ShieldCheck size={16} />,
          },
          {
            value: "skills",
            label: zh() ? "技能权限" : "Skills",
            description: zh()
              ? "选中的技能会在 Pet 对话运行时启用，底层操作仍受 Agent 权限限制。"
              : "Selected Skills are enabled for Pet runs and remain bound by Agent permissions.",
            icon: <Sparkles size={16} />,
            count: selectedSkillIds().length,
          },
          {
            value: "review",
            label: zh() ? "配置摘要" : "Review",
            description: zh()
              ? "保存前检查 Pet Agent 的长期授权范围。"
              : "Review Pet Agent's persistent authorization before saving.",
            icon: <History size={16} />,
          },
        ]}
        value={section()}
        disabled={loading() || saving()}
        onChange={setSection}
        footer={
          <>
            <span class="authorization-workspace-footer-status">
              {permissionLevelLabel(draft().level, zh())} · {selectedSkillIds().length}{" "}
              {zh() ? "个技能" : "Skills"}
            </span>
            <div class="authorization-workspace-footer-actions">
              <Button
                variant="primary"
                data-testid="pet-permission-save"
                disabled={loading() || saving() || !dirty()}
                loading={saving()}
                onClick={() => void save()}
              >
                <ShieldCheck size={16} />
                {zh() ? "保存权限" : "Save permissions"}
              </Button>
            </div>
          </>
        }
      >
        <div class="authorization-workspace-panel" hidden={section() !== "permissions"}>
          <PermissionPolicyEditor
            value={draft()}
            testId="pet-permission"
            zh={zh()}
            disabled={loading() || saving()}
            onChange={update}
          />
        </div>
        <div class="authorization-workspace-panel" hidden={section() !== "skills"}>
          <SkillPermissionEditor
            skills={skills()}
            selectedIds={selectedSkillIds()}
            zh={zh()}
            disabled={loading() || saving()}
            onChange={updateSkills}
          />
        </div>
        <div class="authorization-workspace-panel" hidden={section() !== "review"}>
          <div class="authorization-review-list">
            <div class="authorization-review-row">
              <span>{zh() ? "配置对象" : "Profile"}</span>
              <strong>Pet Agent</strong>
            </div>
            <div class="authorization-review-row">
              <span>{zh() ? "Agent 权限" : "Agent permissions"}</span>
              <strong>{permissionLevelLabel(draft().level, zh())}</strong>
            </div>
            <div class="authorization-review-row">
              <span>{zh() ? "技能" : "Skills"}</span>
              <strong>
                {skills()
                  .filter((skill) => selectedSkillIds().includes(skill.id))
                  .map((skill) => skillDisplayName(skill, zh()))
                  .join(zh() ? "、" : ", ") || (zh() ? "未选择" : "None")}
              </strong>
            </div>
          </div>
        </div>
      </AuthorizationWorkspace>
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

function permissionLevelLabel(level: AgentPermissionPolicy["level"], zh: boolean): string {
  if (level === "full_access") return zh ? "完全授权" : "Full access";
  if (level === "writable") return zh ? "可写" : "Writable";
  return zh ? "只读" : "Read only";
}
