import type { FeatureFlags, RuntimeFeatureSet } from "@hachimi/contracts";
import { describe, expect, it } from "vitest";

import { runtimeFeatureVisibility } from "./runtime-feature-visibility";

const enabledRuntime: RuntimeFeatureSet = {
  runRecovery: true,
  providerExtensions: true,
  providerRemoteContext: true,
  multiAgent: true,
  gitRemoteMutations: true,
  pluginRuntime: true,
  enterpriseIntegrations: true,
};

function flags(runtimeFeatures: RuntimeFeatureSet = enabledRuntime): FeatureFlags {
  return {
    workbench: true,
    motionLab: false,
    workspaceTools: true,
    browserControl: true,
    computerObserve: true,
    computerControl: true,
    remoteTts: false,
    remoteGateway: false,
    pluginRuntime: true,
    localGateway: true,
    mcpRuntime: true,
    scheduler: true,
    runtimeFeatures,
  };
}

describe("runtimeFeatureVisibility", () => {
  it("exposes every release feature when all kill switches are enabled", () => {
    expect(runtimeFeatureVisibility(flags())).toEqual({
      runRecovery: true,
      providerExtensions: true,
      providerRemoteContext: true,
      multiAgent: true,
      gitRemoteMutations: true,
      pluginRuntime: true,
      enterpriseIntegrations: true,
    });
  });

  it("hides each release surface when its runtime feature is disabled", () => {
    for (const key of Object.keys(enabledRuntime) as (keyof RuntimeFeatureSet)[]) {
      const visibility = runtimeFeatureVisibility(flags({ ...enabledRuntime, [key]: false }));
      expect(visibility[key]).toBe(false);
    }
  });

  it("keeps dependent Provider and enterprise surfaces fail-closed", () => {
    expect(
      runtimeFeatureVisibility(flags({ ...enabledRuntime, providerExtensions: false }))
        .providerRemoteContext,
    ).toBe(false);
    expect(runtimeFeatureVisibility({ ...flags(), pluginRuntime: false })).toMatchObject({
      pluginRuntime: false,
      enterpriseIntegrations: false,
    });
  });
});
