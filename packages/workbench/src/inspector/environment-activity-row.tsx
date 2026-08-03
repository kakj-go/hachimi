import type { EnvironmentActivity } from "@hachimi/contracts";
import { Bot, Button, Globe } from "@hachimi/ui";

export function EnvironmentActivityRow(props: {
  activity: EnvironmentActivity;
  locale: "zh-CN" | "en-US";
  onOpenPlan: (planId: string) => void;
  onOpenBrowser: () => void;
}) {
  const zh = () => props.locale === "zh-CN";
  if (props.activity.kind === "browser") {
    const activity = props.activity;
    return (
      <Button
        class="environment-summary-row"
        data-testid="workbench-summary-browser-activity"
        title={activity.domain}
        onClick={props.onOpenBrowser}
      >
        <Globe size={16} />
        <span>{zh() ? `正在访问 ${activity.domain}` : `Visiting ${activity.domain}`}</span>
        <span class="environment-row-tail">›</span>
      </Button>
    );
  }
  const activity = props.activity;
  return (
    <Button
      class="environment-summary-row"
      data-testid="workbench-summary-plan-activity"
      title={activity.description}
      onClick={() => props.onOpenPlan(activity.plan_id)}
    >
      <Bot size={16} />
      <span>{activity.description}</span>
      <span class="environment-row-tail">›</span>
    </Button>
  );
}
