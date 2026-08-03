import {
  commandFailure,
  type CheckoutKind,
  type GitPushResponse,
  type WorkbenchEnvironmentSnapshot,
  type WorkbenchGitAction,
  type WorkbenchGitResponse,
  type WorkbenchHandoffResponse,
  type WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import {
  type Accessor,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  untrack,
} from "solid-js";

import { runMutationContext } from "../mutation-context";
import type { WorkbenchCommandPort } from "../workbench-command-port";

export type WorkbenchEnvironmentController = ReturnType<
  typeof createWorkbenchEnvironmentController
>;

export function createWorkbenchEnvironmentController(options: {
  snapshot: Accessor<WorkbenchSessionSnapshot>;
  commandPort: WorkbenchCommandPort;
  onHandoff?: (response: WorkbenchHandoffResponse) => void;
}) {
  const [environment, setEnvironment] = createSignal<WorkbenchEnvironmentSnapshot>();
  const [loading, setLoading] = createSignal(false);
  const [failure, setFailure] = createSignal<string>();
  let loadGeneration = 0;
  let refreshTimer: ReturnType<typeof setTimeout> | undefined;

  const sessionId = createMemo(() => options.snapshot().session.id);
  const latestRun = createMemo(() => options.snapshot().runs.at(-1));
  const watchBinding = createMemo(() => {
    const current = environment();
    return current ? `${current.sessionId}:${current.checkout.id}` : undefined;
  });

  function applySnapshot(next: WorkbenchEnvironmentSnapshot) {
    if (next.sessionId !== sessionId()) return;
    setEnvironment((current) => {
      if (current && next.revision < current.revision) return current;
      if (
        current &&
        next.revision === current.revision &&
        next.generatedAtMs < current.generatedAtMs
      ) {
        return current;
      }
      return next;
    });
  }

  async function refresh() {
    const requestedSessionId = sessionId();
    const generation = ++loadGeneration;
    setLoading(true);
    try {
      const next = await options.commandPort.getWorkbenchEnvironment(requestedSessionId);
      if (generation === loadGeneration && requestedSessionId === sessionId()) {
        applySnapshot(next);
        setFailure(undefined);
      }
    } catch (error) {
      if (generation === loadGeneration) setFailure(commandFailure(error).message);
    } finally {
      if (generation === loadGeneration) setLoading(false);
    }
  }

  function scheduleRefresh(delay = 150) {
    if (refreshTimer) clearTimeout(refreshTimer);
    refreshTimer = setTimeout(() => {
      refreshTimer = undefined;
      void refresh();
    }, delay);
  }

  async function executeGit(
    action: WorkbenchGitAction,
    includeUnstaged: boolean,
  ): Promise<WorkbenchGitResponse> {
    const current = environment();
    const run = latestRun();
    if (!current || !run) throw new Error("workbench_git_context_missing");
    const context = runMutationContext(run);
    const response = await options.commandPort.executeWorkbenchGit({
      context,
      idempotencyKey: context.idempotencyKey,
      sessionId: current.sessionId,
      checkoutId: current.checkout.id,
      expectedHead: current.git.headSha,
      statusFingerprint: current.git.statusFingerprint,
      includeUnstaged,
      action,
    });
    await refresh();
    return response;
  }

  async function pushGit(input?: {
    remoteName?: string;
    head?: string | null;
    branch?: string | null;
  }): Promise<GitPushResponse> {
    const current = environment();
    const run = latestRun();
    if (!current || !run) throw new Error("workbench_push_context_missing");
    const head = input?.head ?? current.git.headSha;
    const branch = input?.branch ?? current.git.branch;
    const remote =
      current.git.remotes.find((candidate) => candidate.name === input?.remoteName) ??
      defaultRemote(current);
    if (!head || !branch || current.git.detached || !remote) {
      throw new Error("workbench_push_unavailable");
    }
    const context = runMutationContext(run);
    const response = await options.commandPort.pushGitRemote({
      context,
      sessionId: current.sessionId,
      checkoutId: current.checkout.id,
      remoteName: remote.name,
      expectedRemoteUrlHash: remote.remoteUrlHash,
      sourceRef: "HEAD",
      targetRef: `refs/heads/${branch}`,
      expectedCommitOid: head,
      approvalId: null,
    });
    await refresh();
    return response;
  }

  async function handoff(targetKind: CheckoutKind): Promise<WorkbenchHandoffResponse> {
    const current = environment();
    if (!current) throw new Error("workbench_handoff_context_missing");
    const response = await options.commandPort.handoffWorkbenchSession({
      idempotencyKey: crypto.randomUUID(),
      sessionId: current.sessionId,
      sourceCheckoutId: current.checkout.id,
      targetKind,
      expectedHead: current.git.headSha,
      statusFingerprint: current.git.statusFingerprint,
      expectedBindingRevision: current.bindingRevision,
    });
    applySnapshot(response.environment);
    options.onHandoff?.(response);
    return response;
  }

  createEffect(() => {
    const selectedSessionId = sessionId();
    loadGeneration += 1;
    setEnvironment(undefined);
    setFailure(undefined);
    void refresh();
    let disposed = false;
    let stopEnvironment: (() => void) | undefined;
    void options.commandPort
      .onEnvironmentChange((event) => {
        if (event.sessionId !== selectedSessionId) return;
        const currentRevision = untrack(environment)?.revision ?? 0;
        if (event.revision >= currentRevision) scheduleRefresh(0);
      })
      .then((stop) => {
        if (disposed) stop();
        else stopEnvironment = stop;
      })
      .catch((error) => {
        if (!disposed) setFailure(commandFailure(error).message);
      });
    onCleanup(() => {
      disposed = true;
      stopEnvironment?.();
    });
  });

  createEffect(() => {
    const binding = watchBinding();
    if (!binding) return;
    const current = untrack(environment);
    if (!current) return;
    let disposed = false;
    let watchId: string | undefined;
    let stopWorkspace: (() => void) | undefined;
    void options.commandPort
      .onWorkspaceChange((event) => {
        if (event.watchId === watchId) scheduleRefresh();
      })
      .then((stop) => {
        if (disposed) stop();
        else stopWorkspace = stop;
      })
      .catch((error) => {
        if (!disposed) setFailure(commandFailure(error).message);
      });
    void options.commandPort
      .watchWorkspaceFiles({
        sessionId: current.sessionId,
        checkoutId: current.checkout.id,
        path: "",
        recursive: true,
      })
      .then((registration) => {
        if (disposed) {
          void options.commandPort.unwatchWorkspaceFiles(registration.id).catch(() => false);
        } else {
          watchId = registration.id;
        }
      })
      .catch((error) => {
        if (!disposed) setFailure(commandFailure(error).message);
      });
    onCleanup(() => {
      disposed = true;
      stopWorkspace?.();
      if (watchId) void options.commandPort.unwatchWorkspaceFiles(watchId).catch(() => false);
    });
  });

  onCleanup(() => {
    if (refreshTimer) clearTimeout(refreshTimer);
  });

  return {
    environment,
    loading,
    failure,
    latestRun,
    refresh,
    executeGit,
    pushGit,
    handoff,
  };
}

function defaultRemote(environment: WorkbenchEnvironmentSnapshot) {
  const upstreamRemote = environment.git.upstream?.split("/", 1)[0];
  return (
    environment.git.remotes.find((remote) => remote.name === upstreamRemote) ??
    environment.git.remotes[0]
  );
}
