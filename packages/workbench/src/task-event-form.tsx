import type { ScheduleEventSourceKind } from "@hachimi/contracts";
import { SelectField, TextArea, TextField } from "@hachimi/ui";

export function TaskEventForm(props: {
  zh: boolean;
  sourceKind: ScheduleEventSourceKind;
  sourcePrincipal: string;
  sourceId: string;
  eventType: string;
  subjectPrefix: string;
  labels: string;
  resourceKind: string;
  resourceId: string;
  resourceRevision: string;
  onSourceKind: (value: ScheduleEventSourceKind) => void;
  onSourcePrincipal: (value: string) => void;
  onSourceId: (value: string) => void;
  onEventType: (value: string) => void;
  onSubjectPrefix: (value: string) => void;
  onLabels: (value: string) => void;
  onResourceKind: (value: string) => void;
  onResourceId: (value: string) => void;
  onResourceRevision: (value: string) => void;
}) {
  return (
    <section class="task-event-form" data-testid="task-event-form">
      <div class="task-extension-heading">
        <strong>{props.zh ? "事件匹配" : "Event matcher"}</strong>
        <small>
          {props.zh
            ? "来源、类型、标签和可选资源引用均为精确匹配；主题使用前缀匹配。事件元数据不会授予额外权限。"
            : "Source, type, labels, and the optional resource reference match exactly; subject uses prefix matching. Event metadata never grants authority."}
        </small>
      </div>
      <div class="task-form-grid">
        <SelectField
          label={props.zh ? "来源类型" : "Source kind"}
          testId="task-event-source-kind"
          value={props.sourceKind}
          options={[
            { value: "workspace", label: "Workspace" },
            { value: "plugin", label: "Plugin" },
            { value: "connector", label: "Connector" },
            { value: "channel", label: "Channel" },
            { value: "gateway", label: "Gateway" },
          ]}
          onChange={(value) => props.onSourceKind(value as ScheduleEventSourceKind)}
        />
        <TextField
          label={props.zh ? "来源 Principal" : "Source principal"}
          testId="task-event-source-principal"
          value={props.sourcePrincipal}
          placeholder="plugin:calendar"
          onInput={(event) => props.onSourcePrincipal(event.currentTarget.value)}
        />
        <TextField
          label={props.zh ? "来源 ID" : "Source ID"}
          testId="task-event-source-id"
          value={props.sourceId}
          placeholder="calendar-primary"
          onInput={(event) => props.onSourceId(event.currentTarget.value)}
        />
        <TextField
          label={props.zh ? "事件类型" : "Event type"}
          testId="task-event-type"
          value={props.eventType}
          placeholder="resource.changed"
          onInput={(event) => props.onEventType(event.currentTarget.value)}
        />
        <TextField
          label={props.zh ? "主题前缀（可选）" : "Subject prefix (optional)"}
          value={props.subjectPrefix}
          placeholder="workspace://project/"
          onInput={(event) => props.onSubjectPrefix(event.currentTarget.value)}
        />
      </div>
      <TextArea
        label={
          props.zh
            ? "标签（每行 key=value，最多 16 个）"
            : "Labels (one key=value per line, max 16)"
        }
        value={props.labels}
        placeholder={"branch=main\nkind=document"}
        onInput={(event) => props.onLabels(event.currentTarget.value)}
      />
      <div class="task-form-grid task-event-resource">
        <TextField
          label={props.zh ? "资源类型（可选）" : "Resource kind (optional)"}
          value={props.resourceKind}
          placeholder="workspace_file"
          onInput={(event) => props.onResourceKind(event.currentTarget.value)}
        />
        <TextField
          label={props.zh ? "资源 ID" : "Resource ID"}
          value={props.resourceId}
          placeholder="docs/notes.md"
          onInput={(event) => props.onResourceId(event.currentTarget.value)}
        />
        <TextField
          label={props.zh ? "资源 revision（可选）" : "Resource revision (optional)"}
          value={props.resourceRevision}
          placeholder="sha256:…"
          onInput={(event) => props.onResourceRevision(event.currentTarget.value)}
        />
      </div>
    </section>
  );
}
