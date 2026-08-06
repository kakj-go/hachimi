import type {
  AgentPermissionPolicy,
  PermissionProfile,
  ScopedPermissionRules,
} from "@hachimi/contracts";
import {
  PermissionPolicyEditor as SharedPermissionPolicyEditor,
  type PermissionPolicyValue,
} from "@hachimi/ui";

export function emptyScopedPermissionRules(): ScopedPermissionRules {
  return {
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
    computer: { observe: false, act: false, targetWindows: [], maxActions: null },
    mcp: [],
    connectors: [],
  };
}

export function createPermissionPolicy(
  level: PermissionProfile = "read_only",
): AgentPermissionPolicy {
  return { level, rules: emptyScopedPermissionRules(), revision: 0 };
}

export function PermissionPolicyEditor(props: {
  value: AgentPermissionPolicy;
  testId?: string;
  disabled?: boolean;
  zh: boolean;
  onChange: (value: AgentPermissionPolicy) => void;
}) {
  return (
    <SharedPermissionPolicyEditor
      value={props.value}
      {...(props.testId ? { testId: props.testId } : {})}
      {...(props.disabled === undefined ? {} : { disabled: props.disabled })}
      zh={props.zh}
      onChange={(value: PermissionPolicyValue) => props.onChange(value as AgentPermissionPolicy)}
    />
  );
}
