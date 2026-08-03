import { I18nProvider } from "@hachimi/i18n";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import type {
  ApprovalRequestRecord,
  ProposedPlan,
  UserInputRequestRecord,
} from "@hachimi/contracts";

import { WorkbenchGate } from "./workbench-gate";

vi.mock("@hachimi/ui", () => {
  const Icon = () => <span aria-hidden="true" />;
  const Button = (props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{props.children}</button>
  );
  return {
    ApprovalCard: (props: {
      children?: JSX.Element;
      actions?: JSX.Element;
      title: string;
      description?: string;
    }) => (
      <article>
        {props.title}
        {props.description}
        {props.children}
        {props.actions}
      </article>
    ),
    Button,
    ArrowRight: Icon,
    CircleHelp: Icon,
    MessageCircle: Icon,
    Play: Icon,
    Send: Icon,
    ShieldCheck: Icon,
    X: Icon,
    TextArea: (props: JSX.TextareaHTMLAttributes<HTMLTextAreaElement>) => <textarea {...props} />,
    TextField: (props: JSX.InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
  };
});

describe("WorkbenchGate", () => {
  it("describes the full approval decision before allowing it", () => {
    const approval = {
      id: "approval-1",
      action: "write_file",
      resource: "packages/workbench/src/home.tsx",
      riskSummary: "This will modify the active workspace.",
      targetHost: "local",
      requiredScopes: ["workspace.write", "process.exec"],
      grantScope: "session",
      usesRemaining: 4,
      requesterPrincipal: "hachimi-agent",
      expiresAtMs: null,
    } as ApprovalRequestRecord;
    const onResolveApproval = vi.fn();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="en-US">
          <WorkbenchGate
            locale="en-US"
            userInput={undefined}
            approval={approval}
            plan={undefined}
            resolvingUserInput={false}
            resolvingApproval={false}
            acceptingPlan={false}
            revisingPlan={false}
            onResolveUserInput={vi.fn()}
            onResolveApproval={onResolveApproval}
            onAcceptPlan={vi.fn()}
            onRevisePlan={vi.fn()}
            onDismissPlan={vi.fn()}
          />
        </I18nProvider>
      ),
      host,
    );

    expect(host.textContent).toContain("Allow the agent to write file?");
    for (const detail of [
      "This will modify the active workspace.",
      "packages/workbench/src/home.tsx",
      "local",
      "workspace.write",
      "process.exec",
      "Valid for the current session",
      "hachimi-agent",
    ]) {
      expect(host.textContent).toContain(detail);
    }
    const allow = [...host.querySelectorAll<HTMLButtonElement>("button")].find((button) =>
      button.textContent?.includes("Allow for session"),
    );
    allow?.click();
    expect(onResolveApproval).toHaveBeenCalledWith(approval, "approved");

    dispose();
    host.remove();
  });

  it("prioritizes user questions over approvals and plans", () => {
    const userInput = {
      id: "input",
      questions: [
        {
          id: "q1",
          header: "Target",
          prompt: "Choose a target",
          options: [
            { label: "Workbench", value: "workbench", description: "Use Workbench" },
            { label: "CLI", value: "cli", description: "Use CLI" },
          ],
          secret: false,
          autoResolutionMs: null,
          defaultAnswer: null,
        },
      ],
      status: "pending",
    } as UserInputRequestRecord;
    const approval = {
      resource: "dangerous resource",
      action: "write",
      riskSummary: "approval marker",
    } as ApprovalRequestRecord;
    const plan = { contentMarkdown: "plan marker", revision: 1 } as ProposedPlan;
    const noop = vi.fn();

    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="en-US">
          <WorkbenchGate
            locale="en-US"
            userInput={userInput}
            approval={approval}
            plan={plan}
            resolvingUserInput={false}
            resolvingApproval={false}
            acceptingPlan={false}
            revisingPlan={false}
            onResolveUserInput={noop}
            onResolveApproval={noop}
            onAcceptPlan={noop}
            onRevisePlan={noop}
            onDismissPlan={noop}
          />
        </I18nProvider>
      ),
      host,
    );

    expect(host.textContent).toContain("Choose a target");
    expect(host.textContent).not.toContain("approval marker");
    expect(host.textContent).not.toContain("plan marker");
    dispose();
    host.remove();
  });

  it("executes, dismisses and submits a textual plan revision", () => {
    const plan = {
      id: "plan-1",
      revision: 3,
      goal: "Align the Workbench",
      steps: [
        { id: "step-1", description: "Align messages", status: "completed" },
        { id: "step-2", description: "Align panels", status: "pending" },
      ],
    } as ProposedPlan;
    const onAcceptPlan = vi.fn();
    const onRevisePlan = vi.fn();
    const onDismissPlan = vi.fn();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="en-US">
          <WorkbenchGate
            locale="en-US"
            userInput={undefined}
            approval={undefined}
            plan={plan}
            resolvingUserInput={false}
            resolvingApproval={false}
            acceptingPlan={false}
            revisingPlan={false}
            onResolveUserInput={vi.fn()}
            onResolveApproval={vi.fn()}
            onAcceptPlan={onAcceptPlan}
            onRevisePlan={onRevisePlan}
            onDismissPlan={onDismissPlan}
          />
        </I18nProvider>
      ),
      host,
    );

    (host.querySelector('[data-testid="workbench-execute-plan"]') as HTMLButtonElement).click();
    expect(onAcceptPlan).toHaveBeenCalledWith(plan);
    (host.querySelector('[data-testid="workbench-dismiss-plan"]') as HTMLButtonElement).click();
    expect(onDismissPlan).toHaveBeenCalledWith(plan);

    (host.querySelector('[data-testid="workbench-revise-plan"]') as HTMLButtonElement).click();
    const field = host.querySelector("textarea") as HTMLTextAreaElement;
    field.value = "Keep the terminal below the composer";
    field.dispatchEvent(new InputEvent("input", { bubbles: true }));
    (
      host.querySelector('[data-testid="workbench-submit-plan-revision"]') as HTMLButtonElement
    ).click();
    expect(onRevisePlan).toHaveBeenCalledWith(plan, "Keep the terminal below the composer");

    dispose();
    host.remove();
  });

  it("shows one question at a time and submits all selected answers", () => {
    const userInput = {
      id: "input-paged",
      questions: [
        {
          id: "scope",
          header: "Scope",
          prompt: "Choose a scope",
          options: [
            { label: "Workbench", value: "workbench", description: "All panels" },
            { label: "Timeline", value: "timeline", description: "Messages only" },
          ],
          secret: false,
          autoResolutionMs: null,
          defaultAnswer: "workbench",
        },
        {
          id: "density",
          header: "Density",
          prompt: "Choose a density",
          options: [
            { label: "Compact", value: "compact", description: "More activity" },
            { label: "Comfortable", value: "comfortable", description: "More space" },
          ],
          secret: false,
          autoResolutionMs: null,
          defaultAnswer: "compact",
        },
      ],
      status: "pending",
    } as UserInputRequestRecord;
    const onResolveUserInput = vi.fn();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="en-US">
          <WorkbenchGate
            locale="en-US"
            userInput={userInput}
            approval={undefined}
            plan={undefined}
            resolvingUserInput={false}
            resolvingApproval={false}
            acceptingPlan={false}
            revisingPlan={false}
            onResolveUserInput={onResolveUserInput}
            onResolveApproval={vi.fn()}
            onAcceptPlan={vi.fn()}
            onRevisePlan={vi.fn()}
            onDismissPlan={vi.fn()}
          />
        </I18nProvider>
      ),
      host,
    );

    expect(host.textContent).toContain("Choose a scope");
    expect(host.textContent).not.toContain("Choose a density");
    expect(host.querySelectorAll('[role="radiogroup"]')).toHaveLength(1);
    expect(host.textContent).toContain("1 of 2");

    const timelineChoice = [...host.querySelectorAll<HTMLButtonElement>('[role="radio"]')].find(
      (button) => button.textContent?.includes("Timeline"),
    );
    timelineChoice?.click();
    expect(host.textContent).not.toContain("Choose a scope");
    expect(host.textContent).toContain("Choose a density");
    expect(host.textContent).toContain("2 of 2");

    const comfortableChoice = [...host.querySelectorAll<HTMLButtonElement>('[role="radio"]')].find(
      (button) => button.textContent?.includes("Comfortable"),
    );
    comfortableChoice?.click();
    (
      host.querySelector('[data-testid="workbench-submit-user-input"]') as HTMLButtonElement
    ).click();
    expect(onResolveUserInput).toHaveBeenCalledWith(
      userInput,
      [
        { questionId: "scope", value: "timeline" },
        { questionId: "density", value: "comfortable" },
      ],
      "submit",
    );

    dispose();
    host.remove();
  });
});
