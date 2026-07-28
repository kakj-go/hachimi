import type { SandboxCapabilityReport, SandboxRuntimeSnapshot } from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SandboxReadinessBanner } from "./sandbox-readiness-banner";
import type { WorkbenchCommandPort } from "./workbench-command-port";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  return {
    AlertTriangle: Icon,
    RefreshCw: Icon,
    ShieldCheck: Icon,
    Button: (props: Record<string, unknown>) => (
      <button
        type={(props.type as "button" | "submit" | undefined) ?? "button"}
        data-testid={props["data-testid"] as string | undefined}
        disabled={props.disabled as boolean | undefined}
        onClick={(event) =>
          (props.onClick as ((event: MouseEvent) => void) | undefined)?.(event as MouseEvent)
        }
      >
        {props.children as never}
      </button>
    ),
  };
});

function report(enforced: boolean): SandboxCapabilityReport {
  return {
    backend: "windows_sandbox_v1",
    readiness: enforced ? "ready" : "setup_required",
    osEnforced: enforced,
    filesystemEnforced: enforced,
    processEnforced: enforced,
    networkEnforced: enforced,
    version: enforced ? "1" : null,
    stableErrorCode: enforced ? null : "sandbox_setup_required",
    diagnostics: enforced ? [] : ["setup marker is missing"],
  };
}

function snapshot(revision: number, enforced: boolean): SandboxRuntimeSnapshot {
  return { revision, report: report(enforced), repairing: false };
}

async function settle() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe("SandboxReadinessBanner", () => {
  it("loads live readiness and hides only after repair returns an attested snapshot", async () => {
    let finishRepair: ((value: SandboxRuntimeSnapshot) => void) | undefined;
    const repairSandbox = vi.fn(
      () =>
        new Promise<SandboxRuntimeSnapshot>((resolve) => {
          finishRepair = resolve;
        }),
    );
    const port = {
      getSandboxStatus: vi.fn(async () => snapshot(2, false)),
      refreshSandboxStatus: vi.fn(async () => snapshot(3, false)),
      repairSandbox,
    } as unknown as WorkbenchCommandPort;
    const failure = vi.fn();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <SandboxReadinessBanner
            commandPort={port}
            initialReport={report(false)}
            onFailure={failure}
          />
        </I18nProvider>
      ),
      root,
    );

    await settle();
    expect(port.getSandboxStatus).toHaveBeenCalledOnce();
    expect(root.querySelector('[data-testid="sandbox-readiness-banner"]')).not.toBeNull();
    expect(root.textContent).toContain("sandbox_setup_required");
    expect(root.textContent).toContain("setup marker is missing");

    const refresh = [...root.querySelectorAll("button")].find((button) =>
      button.textContent?.includes("刷新"),
    );
    refresh?.click();
    await settle();
    expect(port.refreshSandboxStatus).toHaveBeenCalledOnce();

    const repair = root.querySelector<HTMLButtonElement>('[data-testid="sandbox-repair"]');
    repair?.click();
    expect(repair?.disabled).toBe(true);
    expect(root.textContent).toContain("等待 UAC");
    finishRepair?.(snapshot(4, true));
    await settle();

    expect(repairSandbox).toHaveBeenCalledOnce();
    expect(failure).toHaveBeenLastCalledWith(undefined);
    expect(root.querySelector('[data-testid="sandbox-readiness-banner"]')).toBeNull();
    dispose();
  });
});
