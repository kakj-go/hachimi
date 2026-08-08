import type { ProviderProtocolKind, ReasoningSummaryMode } from "@hachimi/contracts";

export interface RemoteContextFields {
  reasoningSummary: ReasoningSummaryMode;
  remoteCompaction: boolean;
}

export function normalizeRemoteContextFields(
  protocol: ProviderProtocolKind,
  providerRemoteContextEnabled: boolean,
  reasoningSummary: ReasoningSummaryMode,
  remoteCompaction: boolean,
): RemoteContextFields {
  if (protocol !== "responses" || !providerRemoteContextEnabled) {
    return { reasoningSummary: "none", remoteCompaction: false };
  }
  return { reasoningSummary, remoteCompaction };
}
