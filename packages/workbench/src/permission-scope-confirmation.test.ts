import { describe, expect, it } from "vitest";
import type { AgentPermissionPolicy } from "@hachimi/contracts";

import { permissionScopeRisk } from "./permission-scope-risk";

function policy(): AgentPermissionPolicy {
  return {
    level: "writable",
    revision: 0,
    rules: {
      fileSystem: [],
      fileSystemUnrestrictedRead: false,
      fileSystemUnrestrictedWrite: false,
      process: {
        spawn: false,
        interactive: false,
        unrestrictedCommands: false,
        allowedCommands: [],
      },
      network: { enabled: false, unrestrictedHosts: false, hosts: [], protocols: [] },
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
    },
  };
}

describe("permissionScopeRisk", () => {
  it("does not treat a range switch as its related capability", () => {
    const value = policy();
    value.rules.computer.unrestrictedTargets = true;
    expect(permissionScopeRisk(value)).toEqual({
      hasUnrestrictedScope: true,
      equivalentToFullAccess: false,
    });
    expect(value.rules.computer.observe).toBe(false);
    expect(value.rules.computer.act).toBe(false);
  });

  it("requires every scope and capability before reporting full-access equivalence", () => {
    const value = policy();
    value.rules.fileSystemUnrestrictedRead = true;
    value.rules.fileSystemUnrestrictedWrite = true;
    value.rules.process = {
      ...value.rules.process,
      spawn: true,
      interactive: true,
      unrestrictedCommands: true,
    };
    value.rules.network = {
      ...value.rules.network,
      enabled: true,
      unrestrictedHosts: true,
    };
    value.rules.browser = {
      ...value.rules.browser,
      observe: true,
      act: true,
      upload: true,
      download: true,
      cookieStorage: true,
      cdp: true,
      unrestrictedOrigins: true,
    };
    value.rules.computer = {
      ...value.rules.computer,
      observe: true,
      act: true,
      unrestrictedTargets: true,
    };
    expect(permissionScopeRisk(value).equivalentToFullAccess).toBe(true);
  });
});
