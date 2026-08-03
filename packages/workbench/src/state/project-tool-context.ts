import { commandFailure, type WorkbenchSessionSnapshot } from "@hachimi/contracts";
import { createSignal } from "solid-js";

import type { WorkbenchCommandPort } from "../workbench-command-port";

export function createProjectToolContext(
  commandPort: WorkbenchCommandPort,
  onFailure: (message: string) => void,
) {
  const [value, setValue] = createSignal<{
    projectId: string;
    snapshot: WorkbenchSessionSnapshot;
  }>();
  const [loadingProjectId, setLoadingProjectId] = createSignal<string>();
  let generation = 0;
  let pending:
    | { projectId: string; promise: Promise<WorkbenchSessionSnapshot | undefined> }
    | undefined;

  async function ensure(projectId: string): Promise<WorkbenchSessionSnapshot | undefined> {
    const cached = value();
    if (cached?.projectId === projectId) return cached.snapshot;
    if (pending?.projectId === projectId) return pending.promise;
    const requestGeneration = ++generation;
    setLoadingProjectId(projectId);
    const promise = commandPort
      .getWorkbenchProjectToolContext(projectId)
      .then((snapshot) => {
        if (generation === requestGeneration) setValue({ projectId, snapshot });
        return generation === requestGeneration ? snapshot : undefined;
      })
      .catch((error) => {
        if (generation === requestGeneration) onFailure(commandFailure(error).message);
        return undefined;
      })
      .finally(() => {
        if (pending?.promise === promise) pending = undefined;
        if (generation === requestGeneration) setLoadingProjectId(undefined);
      });
    pending = { projectId, promise };
    return promise;
  }

  function clearUnless(projectId: string | undefined) {
    if (value()?.projectId === projectId || loadingProjectId() === projectId) return;
    generation += 1;
    pending = undefined;
    setValue(undefined);
    setLoadingProjectId(undefined);
  }

  return {
    ensure,
    clearUnless,
    loading: (projectId: string | undefined) => loadingProjectId() === projectId,
    snapshot: (projectId: string | undefined) =>
      value()?.projectId === projectId ? value()?.snapshot : undefined,
  };
}
