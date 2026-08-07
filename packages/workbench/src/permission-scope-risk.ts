import type { AgentPermissionPolicy } from "@hachimi/contracts";

export interface PermissionScopeRisk {
  hasUnrestrictedScope: boolean;
  equivalentToFullAccess: boolean;
}

export function permissionScopeRisk(policy: AgentPermissionPolicy): PermissionScopeRisk {
  if (policy.level === "full_access") {
    return { hasUnrestrictedScope: false, equivalentToFullAccess: true };
  }
  const rules = policy.rules;
  const scopes = [
    Boolean(rules.fileSystemUnrestrictedRead),
    Boolean(rules.fileSystemUnrestrictedWrite),
    Boolean(rules.process.unrestrictedCommands),
    Boolean(rules.network.unrestrictedHosts),
    Boolean(rules.browser.unrestrictedOrigins),
    Boolean(rules.computer.unrestrictedTargets),
  ];
  return {
    hasUnrestrictedScope: scopes.some(Boolean),
    equivalentToFullAccess: Boolean(
      scopes.every(Boolean) &&
      rules.process.spawn &&
      rules.process.interactive &&
      rules.network.enabled &&
      rules.browser.observe &&
      rules.browser.act &&
      rules.browser.upload &&
      rules.browser.download &&
      rules.browser.cookieStorage &&
      rules.browser.cdp &&
      rules.computer.observe &&
      rules.computer.act,
    ),
  };
}
