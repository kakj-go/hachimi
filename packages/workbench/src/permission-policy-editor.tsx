import { commands, type ComputerAppCandidate } from "@hachimi/contracts";
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
    fileSystemUnrestrictedRead: false,
    fileSystemUnrestrictedWrite: false,
    network: {
      enabled: false,
      unrestrictedHosts: false,
      hosts: [],
      protocols: [],
    },
    process: {
      spawn: false,
      interactive: false,
      unrestrictedCommands: false,
      allowedCommands: [],
    },
    browser: {
      observe: false,
      act: false,
      upload: false,
      download: false,
      cookieStorage: false,
      cdp: false,
      unrestrictedOrigins: false,
      origins: [],
    },
    computer: {
      observe: false,
      act: false,
      unrestrictedTargets: false,
      allowedApplications: [],
      maxActions: null,
    },
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
  async function listApplications() {
    const [candidates, policies] = await Promise.all([
      commands.listComputerAppCandidates(),
      commands.listComputerAppPolicies(),
    ]);
    const merged = new Map<string, ComputerAppCandidate>();
    candidates.forEach((candidate) => merged.set(candidate.app.identityHash, candidate));
    policies.forEach((policy) => {
      if (!merged.has(policy.app.identityHash)) {
        merged.set(policy.app.identityHash, {
          app: policy.app,
          windowCount: 0,
          iconPngBase64: null,
        });
      }
    });
    return [...merged.values()].map((candidate) => ({
      identityHash: candidate.app.identityHash,
      displayName: candidate.app.displayName,
      executableName: candidate.app.executableName,
      executablePath: candidate.app.executablePath,
      iconPngBase64: candidate.iconPngBase64,
      windowCount: candidate.windowCount ?? 0,
    }));
  }

  async function chooseForegroundApplication() {
    const candidate = await commands.choosePermissionForegroundApplication();
    return candidate
      ? {
          identityHash: candidate.app.identityHash,
          displayName: candidate.app.displayName,
          executableName: candidate.app.executableName,
          executablePath: candidate.app.executablePath,
          iconPngBase64: candidate.iconPngBase64,
          windowCount: candidate.windowCount ?? 0,
        }
      : null;
  }

  return (
    <SharedPermissionPolicyEditor
      value={props.value}
      {...(props.testId ? { testId: props.testId } : {})}
      {...(props.disabled === undefined ? {} : { disabled: props.disabled })}
      zh={props.zh}
      onChange={(value: PermissionPolicyValue) => props.onChange(value as AgentPermissionPolicy)}
      chooseDirectory={() => commands.choosePermissionDirectory()}
      chooseFiles={(root) => commands.choosePermissionFiles(root)}
      searchCommands={(prefix) => commands.searchPermissionCommands(prefix)}
      listApplications={listApplications}
      chooseForegroundApplication={chooseForegroundApplication}
    />
  );
}
