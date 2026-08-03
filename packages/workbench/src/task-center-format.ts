import type {
  ProjectRecord,
  ScheduleDefinition,
  ScheduleEventReceipt,
  TaskRunRecord,
} from "@hachimi/contracts";

export function formatTaskTime(value: number | null, zh?: boolean): string {
  return value
    ? new Intl.DateTimeFormat(zh === undefined ? undefined : zh ? "zh-CN" : "en-US", {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(value)
    : "-";
}

export function formatTaskDuration(run: TaskRunRecord, zh: boolean): string {
  if (run.startedAtMs === null) return "-";
  const active = ["preparing", "running", "needs_attention"].includes(run.status);
  const end = run.finishedAtMs ?? (active ? Date.now() : run.updatedAtMs);
  const durationMs = Math.max(0, end - run.startedAtMs);
  const totalSeconds = Math.max(1, Math.round(durationMs / 1_000));
  const hours = Math.floor(totalSeconds / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  const seconds = totalSeconds % 60;

  if (zh) {
    return [hours ? `${hours} 小时` : "", minutes ? `${minutes} 分` : "", `${seconds} 秒`]
      .filter(Boolean)
      .join(" ");
  }
  return [hours ? `${hours} hr` : "", minutes ? `${minutes} min` : "", `${seconds} sec`]
    .filter(Boolean)
    .join(" ");
}

export function scheduleTimingLabel(schedule: ScheduleDefinition, zh: boolean): string {
  if (schedule.schedule.kind === "event") {
    return schedule.enabled
      ? zh
        ? "等待匹配事件"
        : "Waiting for event"
      : zh
        ? "已停用"
        : "Disabled";
  }
  if (!schedule.enabled) return zh ? "已停用" : "Disabled";
  return `${zh ? "下次执行" : "Next run"} ${formatTaskTime(schedule.nextRunAtMs, zh)}`;
}

export function scheduleFrequencyLabel(schedule: ScheduleDefinition, zh: boolean): string {
  const spec = schedule.schedule;
  if (spec.kind === "event") return zh ? "事件触发" : "Event trigger";
  if (spec.kind === "at") {
    return `${zh ? "执行于" : "Runs"} ${formatTaskTime(spec.timestamp_ms, zh)}`;
  }
  if (spec.kind === "every") {
    const minutes = Math.max(1, Math.round(spec.interval_ms / 60_000));
    return zh ? `每 ${minutes} 分钟` : `Every ${minutes} min`;
  }
  return `${spec.expression} · ${spec.timezone}`;
}

export function taskContextLabel(
  schedule: ScheduleDefinition,
  projects: ProjectRecord[],
  zh: boolean,
): string {
  if (schedule.contextTemplate.kind === "general") return zh ? "通用" : "General";
  if (schedule.contextTemplate.kind === "session_continuation") {
    return zh ? "续接现有会话" : "Continue a session";
  }
  const projectId = schedule.contextTemplate.project_id;
  return projects.find((project) => project.id === projectId)?.displayName ?? projectId;
}

export function scheduleHealthTone(
  health: ScheduleDefinition["health"],
): "neutral" | "success" | "warning" | "danger" {
  if (health === "healthy") return "success";
  if (health === "invalid") return "danger";
  return "warning";
}

export function scheduleHealthLabel(health: ScheduleDefinition["health"], zh: boolean): string {
  const labels = {
    healthy: zh ? "正常" : "Healthy",
    needs_authorization: zh ? "需要授权" : "Authorization required",
    needs_attention: zh ? "需要处理" : "Needs attention",
    invalid: zh ? "配置无效" : "Invalid",
  } satisfies Record<ScheduleDefinition["health"], string>;
  return labels[health];
}

export function taskRunTone(
  status: TaskRunRecord["status"],
): "neutral" | "info" | "success" | "warning" | "danger" {
  if (status === "succeeded") return "success";
  if (["queued", "preparing", "running"].includes(status)) return "info";
  if (["needs_attention", "skipped"].includes(status)) return "warning";
  if (["failed", "timed_out", "cancelled", "lost"].includes(status)) return "danger";
  return "neutral";
}

export function taskRunStatusLabel(status: TaskRunRecord["status"], zh: boolean): string {
  const labels = {
    queued: zh ? "等待中" : "Queued",
    preparing: zh ? "准备中" : "Preparing",
    running: zh ? "执行中" : "Running",
    needs_attention: zh ? "需要处理" : "Needs attention",
    succeeded: zh ? "已完成" : "Succeeded",
    failed: zh ? "失败" : "Failed",
    timed_out: zh ? "超时" : "Timed out",
    cancelled: zh ? "已取消" : "Cancelled",
    lost: zh ? "已中断" : "Lost",
    skipped: zh ? "已跳过" : "Skipped",
  } satisfies Record<TaskRunRecord["status"], string>;
  return labels[status];
}

export function taskRunTriggerLabel(trigger: TaskRunRecord["trigger"], zh: boolean): string {
  const labels = {
    scheduled: zh ? "计划触发" : "Scheduled",
    manual: zh ? "手动执行" : "Manual",
    retry: zh ? "重试执行" : "Retry",
    catch_up: zh ? "补偿执行" : "Catch-up",
    event: zh ? "事件触发" : "Event",
  } satisfies Record<TaskRunRecord["trigger"], string>;
  return labels[trigger];
}

export function eventReceiptTone(
  status: ScheduleEventReceipt["status"],
): "neutral" | "success" | "warning" | "danger" {
  if (status === "accepted") return "success";
  if (status === "conflict") return "danger";
  return "warning";
}

export function eventReceiptLabel(status: ScheduleEventReceipt["status"], zh: boolean): string {
  if (status === "accepted") return zh ? "已接受" : "Accepted";
  if (status === "conflict") return zh ? "冲突" : "Conflict";
  return zh ? "已重放" : "Replayed";
}
