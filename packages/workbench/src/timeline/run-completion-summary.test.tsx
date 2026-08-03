import type { RunSummaryRecord } from "@hachimi/contracts";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { TimelineRunCompletion } from "./run-completion-summary";

vi.mock("@hachimi/ui", () => ({
  Box: () => <span />,
  Button: (props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props} />,
  ChevronDown: () => <span />,
}));

const emptySummary: RunSummaryRecord = {
  runId: "run-1",
  status: "failed",
  changedFiles: 0,
  additions: 0,
  deletions: 0,
  files: [],
  diffArtifactId: null,
  completedAtMs: 1,
  diffUnavailable: false,
};

afterEach(() => document.body.replaceChildren());

describe("TimelineRunCompletion", () => {
  it("hides failed and successful runs without a real file change", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <TimelineRunCompletion summary={emptySummary} locale="zh-CN" onOpenDiff={() => undefined} />
      ),
      host,
    );
    expect(host.querySelector(".run-completion-summary")).toBeNull();
    dispose();

    const secondDispose = render(
      () => (
        <TimelineRunCompletion
          summary={{ ...emptySummary, status: "succeeded" }}
          locale="zh-CN"
          onOpenDiff={() => undefined}
        />
      ),
      host,
    );
    expect(host.querySelector(".run-completion-summary")).toBeNull();
    secondDispose();
  });

  it("shows a summary whenever a real Git change exists", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const open = vi.fn();
    const dispose = render(
      () => (
        <TimelineRunCompletion
          summary={{
            ...emptySummary,
            changedFiles: 1,
            additions: 3,
            files: [
              {
                path: "src/main.rs",
                previousPath: null,
                status: "modified",
                additions: 3,
                deletions: 0,
                binary: false,
              },
            ],
          }}
          locale="zh-CN"
          onOpenDiff={open}
        />
      ),
      host,
    );
    expect(host.textContent).toContain("已编辑 1 个文件");
    host.querySelectorAll<HTMLButtonElement>("button")[1]!.click();
    expect(open).toHaveBeenCalledWith("run-1", "src/main.rs");
    dispose();
  });
});
