import type { SkillRecord } from "@hachimi/contracts";
import { Button, Checkbox, IconButton, Puzzle, SearchField, X } from "@hachimi/ui";
import { For, Show, createMemo, createSignal } from "solid-js";

import { skillDisplayName } from "./skill-display";

export function SkillPermissionEditor(props: {
  skills: SkillRecord[];
  selectedIds: string[];
  disabled?: boolean;
  zh: boolean;
  onChange: (ids: string[]) => void;
}) {
  const [query, setQuery] = createSignal("");
  const [selectedOnly, setSelectedOnly] = createSignal(false);
  const enabledSkills = createMemo(() => props.skills.filter((skill) => skill.enabled));
  const visibleSkills = createMemo(() => {
    const normalized = query().trim().toLocaleLowerCase();
    return enabledSkills().filter((skill) => {
      const selected = props.selectedIds.includes(skill.id);
      return (
        (!selectedOnly() || selected) &&
        (!normalized ||
          `${skillDisplayName(skill, props.zh)} ${skill.qualifiedName} ${skill.description}`
            .toLocaleLowerCase()
            .includes(normalized))
      );
    });
  });

  function toggle(skillId: string, checked: boolean) {
    props.onChange(
      checked
        ? [...new Set([...props.selectedIds, skillId])]
        : props.selectedIds.filter((id) => id !== skillId),
    );
  }

  return (
    <div class="skill-permission-editor" data-testid="skill-permission-editor">
      <div class="skill-permission-toolbar">
        <SearchField
          label={props.zh ? "搜索技能" : "Search Skills"}
          value={query()}
          disabled={props.disabled}
          placeholder={props.zh ? "搜索技能名称或标识" : "Search name or identifier"}
          onInput={(event) => setQuery(event.currentTarget.value)}
        />
        <div class="skill-permission-actions">
          <Checkbox
            label={props.zh ? "仅看已选" : "Selected only"}
            checked={selectedOnly()}
            disabled={props.disabled}
            onChange={(event) => setSelectedOnly(event.currentTarget.checked)}
          />
          <IconButton
            label={props.zh ? "清空技能" : "Clear Skills"}
            variant="ghost"
            size="small"
            disabled={props.disabled || props.selectedIds.length === 0}
            onClick={() => props.onChange([])}
          >
            <X size={15} />
          </IconButton>
        </div>
      </div>

      <div class="skill-permission-list">
        <For each={visibleSkills()}>
          {(skill) => {
            const selected = () => props.selectedIds.includes(skill.id);
            const displayName = () => skillDisplayName(skill, props.zh);
            return (
              <div class="skill-permission-row" classList={{ selected: selected() }}>
                <Checkbox
                  class="skill-permission-checkbox"
                  label={displayName()}
                  checked={selected()}
                  disabled={props.disabled}
                  onChange={(event) => toggle(skill.id, event.currentTarget.checked)}
                />
                <span class="skill-permission-icon">
                  <Puzzle size={17} />
                </span>
                <span class="skill-permission-copy">
                  <strong>{displayName()}</strong>
                  <code>{skill.qualifiedName}</code>
                </span>
                <small>{skillScopeLabel(skill, props.zh)}</small>
              </div>
            );
          }}
        </For>
        <Show when={visibleSkills().length === 0}>
          <div class="skill-permission-empty">
            <Puzzle size={18} />
            <span>{props.zh ? "没有符合条件的技能" : "No matching Skills"}</span>
            <Show when={query() || selectedOnly()}>
              <Button
                variant="ghost"
                size="small"
                onClick={() => {
                  setQuery("");
                  setSelectedOnly(false);
                }}
              >
                {props.zh ? "重置筛选" : "Reset filters"}
              </Button>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
}

function skillScopeLabel(skill: SkillRecord, zh: boolean) {
  if (skill.scope === "built_in") return zh ? "内置" : "Built-in";
  if (skill.scope === "repo") return zh ? "项目" : "Project";
  if (skill.scope === "system" || skill.scope === "admin") return zh ? "系统" : "System";
  return zh ? "个人" : "Personal";
}
