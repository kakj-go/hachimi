import { I18nProvider } from "@hachimi/i18n";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { describe, expect, it, vi } from "vitest";

import { PendingGateStatus } from "./session-timeline";

vi.mock("@hachimi/ui", () => {
  const Icon = (): JSX.Element => <span aria-hidden="true" />;
  return {
    Archive: Icon,
    Badge: (props: { children: JSX.Element }) => <span>{props.children}</span>,
    Button: (props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) => (
      <button {...props}>{props.children}</button>
    ),
    ChevronDown: Icon,
    CircleHelp: Icon,
    File: Icon,
    Lightbulb: Icon,
    Maximize2: Icon,
    ShieldCheck: Icon,
    TerminalSquare: Icon,
    AgentMessage: (props: { children: JSX.Element }) => <article>{props.children}</article>,
  };
});

describe("PendingGateStatus", () => {
  it.each([
    ["approval", "等待批准", "批准或拒绝后继续"],
    ["plan", "等待确认计划", "实施、修改或跳过"],
    ["user_input", "等待回答", "回答问题后 Agent 将继续"],
  ] as const)("describes the %s wait state in the conversation", (kind, title, detail) => {
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(
      () => (
        <I18nProvider initialLocale="zh-CN">
          <PendingGateStatus kind={kind} />
        </I18nProvider>
      ),
      host,
    );

    expect(host.querySelector('[role="status"]')).not.toBeNull();
    expect(host.textContent).toContain(title);
    expect(host.textContent).toContain(detail);
    dispose();
    host.remove();
  });
});
