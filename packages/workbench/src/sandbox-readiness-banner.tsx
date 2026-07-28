import {
  commandFailure,
  type SandboxCapabilityReport,
  type SandboxRuntimeSnapshot,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { AlertTriangle, Button, RefreshCw, ShieldCheck } from "@hachimi/ui";
import { Show, createSignal, onMount } from "solid-js";

import type { WorkbenchCommandPort } from "./workbench-command-port";

export function SandboxReadinessBanner(props: {
  commandPort: WorkbenchCommandPort;
  initialReport: SandboxCapabilityReport | undefined;
  onFailure: (message: string | undefined) => void;
}) {
  const i18n = useI18n();
  const [snapshot, setSnapshot] = createSignal<SandboxRuntimeSnapshot>();
  const [busy, setBusy] = createSignal<"refresh" | "repair">();
  const report = () => snapshot()?.report ?? props.initialReport;
  const zh = () => i18n.locale() === "zh-CN";

  const refresh = async () => {
    setBusy("refresh");
    props.onFailure(undefined);
    try {
      setSnapshot(await props.commandPort.refreshSandboxStatus());
    } catch (error) {
      props.onFailure(commandFailure(error).message);
    } finally {
      setBusy(undefined);
    }
  };

  const repair = async () => {
    setBusy("repair");
    props.onFailure(undefined);
    try {
      setSnapshot(
        await props.commandPort.repairSandbox({
          context: {
            requestId: crypto.randomUUID(),
            clientId: "window:workbench",
            protocolVersion: 18,
            idempotencyKey: crypto.randomUUID(),
            expectedRunId: null,
            expectedGeneration: null,
          },
        }),
      );
    } catch (error) {
      props.onFailure(commandFailure(error).message);
      try {
        setSnapshot(await props.commandPort.getSandboxStatus());
      } catch {
        // The original repair error is more useful than a follow-up refresh failure.
      }
    } finally {
      setBusy(undefined);
    }
  };

  onMount(() => {
    const onFailure = props.onFailure;
    void props.commandPort
      .getSandboxStatus()
      .then(setSnapshot)
      .catch((error) => onFailure(commandFailure(error).message));
  });

  return (
    <Show when={report() && !report()!.osEnforced}>
      <div
        class="sandbox-readiness-banner composer-notice"
        role="status"
        data-testid="sandbox-readiness-banner"
      >
        <AlertTriangle size={15} />
        <span class="sandbox-readiness-copy">
          <strong>
            {zh()
              ? "Windows Sandbox 尚未通过运行时验证"
              : "Windows Sandbox is not runtime-attested"}
          </strong>
          <small>
            {report()?.stableErrorCode ?? "sandbox_not_enforced"}
            <Show when={report()?.diagnostics[0]}> · {report()!.diagnostics[0]}</Show>
          </small>
        </span>
        <Button
          type="button"
          variant="ghost"
          size="small"
          disabled={Boolean(busy())}
          onClick={() => void refresh()}
        >
          <RefreshCw size={14} classList={{ "is-spinning": busy() === "refresh" }} />
          {zh() ? "刷新" : "Refresh"}
        </Button>
        <Button
          type="button"
          size="small"
          disabled={Boolean(busy())}
          data-testid="sandbox-repair"
          onClick={() => void repair()}
        >
          <ShieldCheck size={14} />
          {busy() === "repair"
            ? zh()
              ? "等待 UAC…"
              : "Waiting for UAC…"
            : zh()
              ? "安装/修复"
              : "Install/repair"}
        </Button>
      </div>
    </Show>
  );
}
