import type { ScheduleDefinition, TaskRunRecord } from "@hachimi/contracts";
import {
  Badge,
  Button,
  CalendarClock,
  Clock3,
  GitBranch,
  History,
  IconButton,
  Pencil,
  Play,
  Switch,
  Tooltip,
  Trash2,
} from "@hachimi/ui";
import { Show } from "solid-js";

import {
  scheduleFrequencyLabel,
  scheduleHealthLabel,
  scheduleHealthTone,
  scheduleTimingLabel,
  formatTaskDuration,
  formatTaskTime,
  taskContextLabel,
  taskRunStatusLabel,
  taskRunTone,
} from "./task-center-format";

export function TaskCard(props: {
  schedule: ScheduleDefinition;
  recentRun: TaskRunRecord | undefined;
  zh: boolean;
  busy: boolean;
  onRun: () => void;
  onHistory: () => void;
  onToggle: (enabled: boolean) => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  return (
    <article
      class="task-card"
      data-testid="task-schedule-card"
      data-schedule-id={props.schedule.id}
      data-schedule-name={props.schedule.name}
      aria-label={props.schedule.name}
    >
      <header class="task-card-header">
        <span class="task-card-icon" aria-hidden="true">
          <CalendarClock size={18} />
        </span>
        <div class="task-card-title">
          <div>
            <h2>{props.schedule.name}</h2>
            <Badge tone={scheduleHealthTone(props.schedule.health)}>
              {scheduleHealthLabel(props.schedule.health, props.zh)}
            </Badge>
          </div>
          <span>{scheduleTimingLabel(props.schedule, props.zh)}</span>
        </div>
        <div class="task-card-hover-actions">
          <Tooltip label={props.zh ? "编辑任务" : "Edit task"}>
            <IconButton
              class="task-card-icon-action"
              size="small"
              variant="ghost"
              label={props.zh ? "编辑任务" : "Edit task"}
              disabled={props.busy}
              data-testid="task-edit"
              onClick={props.onEdit}
            >
              <Pencil size={15} />
            </IconButton>
          </Tooltip>
          <Tooltip label={props.zh ? "删除任务" : "Delete task"}>
            <IconButton
              class="task-card-icon-action task-card-delete"
              size="small"
              variant="ghost"
              tone="danger"
              label={props.zh ? "删除任务" : "Delete task"}
              disabled={props.busy}
              data-testid="task-delete"
              onClick={props.onDelete}
            >
              <Trash2 size={15} />
            </IconButton>
          </Tooltip>
        </div>
      </header>

      <p class="task-card-prompt">{props.schedule.prompt}</p>

      <div class="task-card-meta">
        <span>
          <GitBranch size={14} />
          <span>{taskContextLabel(props.schedule, props.zh)}</span>
        </span>
        <span>
          <Clock3 size={14} />
          <span>{scheduleFrequencyLabel(props.schedule, props.zh)}</span>
        </span>
      </div>

      <div class="task-card-run-state">
        <div class="task-card-run-heading">
          <span>{props.zh ? "最近运行" : "Latest run"}</span>
          <Show
            when={props.recentRun}
            fallback={<span class="task-card-never-run">{props.zh ? "尚未运行" : "Not run"}</span>}
          >
            {(run) => (
              <Badge tone={taskRunTone(run().status)}>
                {taskRunStatusLabel(run().status, props.zh)}
              </Badge>
            )}
          </Show>
        </div>
        <Show
          when={props.recentRun}
          fallback={
            <small>
              {props.zh ? "首次运行后将在这里显示结果" : "Results appear after the first run"}
            </small>
          }
        >
          {(run) => (
            <small>
              {formatTaskTime(run().createdAtMs, props.zh)} · {props.zh ? "用时" : "Duration"}{" "}
              {formatTaskDuration(run(), props.zh)}
            </small>
          )}
        </Show>
      </div>

      <Show when={props.schedule.health !== "healthy"}>
        <div class="task-card-attention">
          <span>
            {props.schedule.healthReason ??
              (props.zh ? "任务设置需要检查" : "Task settings need attention")}
          </span>
          <Button size="small" variant="ghost" disabled={props.busy} onClick={props.onEdit}>
            <Pencil size={14} />
            {props.zh ? "检查设置" : "Review settings"}
          </Button>
        </div>
      </Show>

      <footer class="task-card-footer">
        <div class="task-card-primary-actions">
          <Button
            size="small"
            variant="primary"
            disabled={props.busy}
            data-testid="task-run-now"
            onClick={props.onRun}
          >
            <Play size={15} />
            {props.zh ? "立即执行" : "Run now"}
          </Button>
          <Button size="small" variant="ghost" data-testid="task-history" onClick={props.onHistory}>
            <History size={15} />
            {props.zh ? "历史" : "History"}
          </Button>
        </div>
        <label class="task-card-switch-label">
          <span>
            {props.schedule.enabled
              ? props.zh
                ? "已启用"
                : "Enabled"
              : props.zh
                ? "已停用"
                : "Disabled"}
          </span>
          <Switch
            checked={props.schedule.enabled}
            label={props.zh ? "启用任务" : "Enable task"}
            testId="task-toggle-enabled"
            disabled={props.busy}
            onChange={props.onToggle}
          />
        </label>
      </footer>
    </article>
  );
}
