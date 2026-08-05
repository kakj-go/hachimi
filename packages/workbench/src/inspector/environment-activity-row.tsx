import type { EnvironmentActivity } from "@hachimi/contracts";
import { Bot, Button, Globe, Monitor } from "@hachimi/ui";
import { Match, Switch } from "solid-js";

export function EnvironmentActivityRow(props: {
  activity: EnvironmentActivity;
  locale: "zh-CN" | "en-US";
  onOpenPlan: (planId: string) => void;
  onOpenBrowser: (activity: Extract<EnvironmentActivity, { kind: "browser" }>) => void;
  onOpenComputer: (activity: Extract<EnvironmentActivity, { kind: "computer" }>) => void;
}) {
  const zh = () => props.locale === "zh-CN";
  const browserActivity = (): Extract<EnvironmentActivity, { kind: "browser" }> | undefined =>
    props.activity.kind === "browser" ? props.activity : undefined;
  const computerActivity = (): Extract<EnvironmentActivity, { kind: "computer" }> | undefined =>
    props.activity.kind === "computer" ? props.activity : undefined;
  const planActivity = (): Extract<EnvironmentActivity, { kind: "plan" }> | undefined =>
    props.activity.kind === "plan" ? props.activity : undefined;

  return (
    <Switch>
      <Match when={browserActivity()}>
        {(activity) => (
          <Button
            class="environment-summary-row"
            data-testid="workbench-summary-browser-activity"
            title={activity().domain}
            onClick={() => props.onOpenBrowser(activity())}
          >
            <Globe size={16} />
            <span>{zh() ? `正在访问 ${activity().domain}` : `Visiting ${activity().domain}`}</span>
            <span class="environment-row-tail">›</span>
          </Button>
        )}
      </Match>
      <Match when={computerActivity()}>
        {(activity) => (
          <Button
            class="environment-summary-row"
            data-testid="workbench-summary-computer-activity"
            title={activity().app_name}
            onClick={() => props.onOpenComputer(activity())}
          >
            <Monitor size={16} />
            <span>{activity().app_name}</span>
            <span class="environment-row-tail">›</span>
          </Button>
        )}
      </Match>
      <Match when={planActivity()}>
        {(activity) => (
          <Button
            class="environment-summary-row"
            data-testid="workbench-summary-plan-activity"
            title={activity().description}
            onClick={() => props.onOpenPlan(activity().plan_id)}
          >
            <Bot size={16} />
            <span>{activity().description}</span>
            <span class="environment-row-tail">›</span>
          </Button>
        )}
      </Match>
    </Switch>
  );
}
