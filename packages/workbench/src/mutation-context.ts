import { CONTROL_PROTOCOL_VERSION, type MutationContext, type RunRecord } from "@hachimi/contracts";

type RunFence = Pick<RunRecord, "id" | "generation">;

function baseMutationContext(): Omit<MutationContext, "expectedRunId" | "expectedGeneration"> {
  return {
    requestId: crypto.randomUUID(),
    clientId: "window:workbench",
    protocolVersion: CONTROL_PROTOCOL_VERSION,
    idempotencyKey: crypto.randomUUID(),
  };
}

export function directUserMutationContext(): MutationContext {
  return {
    ...baseMutationContext(),
    expectedRunId: null,
    expectedGeneration: null,
  };
}

export function runMutationContext(run: RunFence): MutationContext {
  return {
    ...baseMutationContext(),
    expectedRunId: run.id,
    expectedGeneration: run.generation,
  };
}
