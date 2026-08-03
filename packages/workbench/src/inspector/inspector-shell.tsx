import type { JSX } from "solid-js";

export function InspectorShell(props: {
  title: string;
  resourceKind?: "tools" | "resource";
  wide?: boolean;
  children: JSX.Element;
}) {
  return (
    <aside
      class="workbench-inspector"
      classList={{ "workbench-inspector-wide": props.wide }}
      data-resource={props.resourceKind ?? "resource"}
      aria-label={props.title}
    >
      <div class="workbench-inspector-body">{props.children}</div>
    </aside>
  );
}
