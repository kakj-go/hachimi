import type {
  ReviewFinding,
  ReviewSnapshot,
  ReviewStartRequest,
  ReviewStartSnapshot,
  SessionRecord,
  WorkbenchSessionSnapshot,
} from "@hachimi/contracts";
import { I18nProvider } from "@hachimi/i18n";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ReviewPanel } from "./review-panel";
import type { WorkbenchCommandPort } from "./workbench-command-port";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  return {
    AlertTriangle: Icon,
    Bug: Icon,
    Check: Icon,
    GitPullRequest: Icon,
    RefreshCw: Icon,
    ShieldCheck: Icon,
    X: Icon,
    Badge: (props: { children?: JSX.Element }) => <span>{props.children}</span>,
    Button: (props: {
      children?: JSX.Element;
      disabled?: boolean;
      loading?: boolean;
      onClick?: () => void;
      class?: string;
      classList?: Record<string, boolean>;
      "data-testid"?: string;
    }) => (
      <button
        type="button"
        class={props.class}
        classList={props.classList}
        data-testid={props["data-testid"]}
        disabled={props.disabled || props.loading}
        onClick={() => props.onClick?.()}
      >
        {props.children}
      </button>
    ),
    SelectField: (props: {
      label: string;
      testId?: string;
      value: string;
      options: readonly { value: string; label: string }[];
      onChange?: (value: string) => void;
    }) => (
      <label>
        {props.label}
        <select
          data-testid={props.testId}
          value={props.value}
          onChange={(event) => props.onChange?.(event.currentTarget.value)}
        >
          <For each={props.options}>
            {(option) => <option value={option.value}>{option.label}</option>}
          </For>
        </select>
      </label>
    ),
    TextField: (props: {
      label: string;
      testId?: string;
      value?: string;
      placeholder?: string;
      onInput?: JSX.EventHandler<HTMLInputElement, InputEvent>;
    }) => (
      <label>
        {props.label}
        <input
          data-testid={props.testId}
          value={props.value ?? ""}
          placeholder={props.placeholder}
          onInput={(event) => props.onInput?.(event)}
        />
      </label>
    ),
  };
});

const sourceSession = {
  id: "session-source",
  context: { kind: "project", project_id: "project-1", checkout_id: "checkout-1" },
  entryProfile: "workbench",
  title: "Source",
  archived: false,
  pinned: false,
  parentSessionId: null,
  sourceRunId: null,
  createdAtMs: 1,
  updatedAtMs: 2,
} as SessionRecord;

function sessionSnapshot(): WorkbenchSessionSnapshot {
  return {
    session: sourceSession,
    runs: [
      {
        id: "run-source",
        sessionId: sourceSession.id,
        status: "succeeded",
        generation: 4,
        updatedAtMs: 10,
      },
    ],
    events: [],
    transcript: [],
    pendingApprovals: [],
    proposedPlans: [],
    artifacts: [],
  } as unknown as WorkbenchSessionSnapshot;
}

function completedReview(status: ReviewFinding["status"] = "open"): ReviewSnapshot {
  return {
    review: {
      id: "review-1",
      sessionId: sourceSession.id,
      runId: "run-review",
      target: { kind: "uncommitted_changes" },
      delivery: "inline",
      createdAtMs: 20,
    },
    run: {
      id: "run-review",
      sessionId: sourceSession.id,
      status: "succeeded",
      generation: 1,
      updatedAtMs: 30,
    },
    findings: [
      {
        id: "finding-1",
        reviewId: "review-1",
        severity: "error",
        file: "src/lib.rs",
        line: 12,
        message: "Cancellation result can win after completion",
        evidence: "The late branch persists a success after the generation was fenced.",
        status,
      },
    ],
    summary: "One actionable concurrency defect.",
    overallCorrectness: "incorrect",
    overallConfidenceScore: 0.94,
  } as unknown as ReviewSnapshot;
}

function startSnapshot(delivery: "inline" | "detached"): ReviewStartSnapshot {
  const session =
    delivery === "inline"
      ? sourceSession
      : ({
          ...sourceSession,
          id: "session-review",
          title: "Review: Source",
          parentSessionId: sourceSession.id,
          sourceRunId: "run-source",
        } as SessionRecord);
  return {
    review: {
      id: "review-1",
      sessionId: session.id,
      runId: "run-review",
      target: { kind: "uncommitted_changes" },
      delivery,
      createdAtMs: 20,
    },
    session,
    run: {
      id: "run-review",
      sessionId: session.id,
      status: "queued",
      generation: 1,
      updatedAtMs: 20,
    },
  } as unknown as ReviewStartSnapshot;
}

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe("ReviewPanel", () => {
  it("starts an inline read-only Review and persists structured finding state", async () => {
    let snapshots: ReviewSnapshot[] = [];
    const startReview = vi.fn(async (request: ReviewStartRequest) => {
      void request;
      snapshots = [completedReview()];
      return startSnapshot("inline");
    });
    const updateReviewFinding = vi.fn(async (request) => ({
      ...completedReview().findings[0]!,
      status: request.status,
    }));
    const port = {
      listReviews: vi.fn(async () => snapshots),
      startReview,
      updateReviewFinding,
    } as unknown as WorkbenchCommandPort;
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <ReviewPanel
            snapshot={sessionSnapshot()}
            commandPort={port}
            onOpenSession={() => undefined}
          />
        </I18nProvider>
      ),
      host,
    );

    [...host.querySelectorAll("button")].find((button) => button.textContent === "打开")?.click();
    (host.querySelector('[data-testid="review-start"]') as HTMLButtonElement).click();

    await vi.waitFor(() => expect(startReview).toHaveBeenCalledTimes(1));
    expect(startReview.mock.calls[0]![0]).toMatchObject({
      sessionId: sourceSession.id,
      target: { kind: "uncommitted_changes" },
      delivery: "inline",
      context: { expectedRunId: "run-source", expectedGeneration: 4 },
    });
    await vi.waitFor(() => expect(host.textContent).toContain("Cancellation result can win"));
    [...host.querySelectorAll("button")]
      .find((button) => button.textContent?.includes("已解决"))
      ?.click();
    await vi.waitFor(() => expect(updateReviewFinding).toHaveBeenCalledTimes(1));
    await vi.waitFor(() =>
      expect(host.querySelector('[data-testid="review-finding"]')?.textContent).toContain(
        "resolved",
      ),
    );
    dispose();
  });

  it("opens a detached Review in its lineage Session", async () => {
    const onOpenSession = vi.fn();
    const port = {
      listReviews: vi.fn(async () => []),
      startReview: vi.fn(async () => startSnapshot("detached")),
    } as unknown as WorkbenchCommandPort;
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <ReviewPanel
            snapshot={sessionSnapshot()}
            commandPort={port}
            onOpenSession={onOpenSession}
          />
        </I18nProvider>
      ),
      host,
    );

    [...host.querySelectorAll("button")].find((button) => button.textContent === "打开")?.click();
    const delivery = host.querySelector('[data-testid="review-delivery"]') as HTMLSelectElement;
    delivery.value = "detached";
    delivery.dispatchEvent(new Event("change", { bubbles: true }));
    (host.querySelector('[data-testid="review-start"]') as HTMLButtonElement).click();

    await vi.waitFor(() => expect(onOpenSession).toHaveBeenCalledTimes(1));
    expect(onOpenSession.mock.calls[0]![0]).toMatchObject({
      id: "session-review",
      parentSessionId: sourceSession.id,
      sourceRunId: "run-source",
    });
    dispose();
  });
});
