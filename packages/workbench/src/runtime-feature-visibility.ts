import type { FeatureFlags } from "@hachimi/contracts";

export type RuntimeFeatureVisibility = {
  runRecovery: boolean;
  providerExtensions: boolean;
  providerRemoteContext: boolean;
  multiAgent: boolean;
  gitRemoteMutations: boolean;
  pluginRuntime: boolean;
  enterpriseIntegrations: boolean;
  desktopControl: boolean;
};

export function runtimeFeatureVisibility(flags: FeatureFlags): RuntimeFeatureVisibility {
  const runtime = flags.runtimeFeatures;
  const pluginRuntime = flags.pluginRuntime && runtime.pluginRuntime;
  return {
    runRecovery: runtime.runRecovery,
    providerExtensions: runtime.providerExtensions,
    providerRemoteContext: runtime.providerExtensions && runtime.providerRemoteContext,
    multiAgent: runtime.multiAgent,
    gitRemoteMutations: runtime.gitRemoteMutations,
    pluginRuntime,
    enterpriseIntegrations: pluginRuntime && runtime.enterpriseIntegrations,
    desktopControl: runtime.desktopControl,
  };
}
