import type { JSX } from "solid-js";

function clamp(value: number, minimum: number, maximum: number) {
  return Math.min(maximum, Math.max(minimum, Math.round(value)));
}

export function WorkbenchResizeHandle(props: {
  orientation: "vertical" | "horizontal";
  value: number;
  minimum: number;
  maximum: number;
  defaultValue: number;
  direction?: 1 | -1;
  label: string;
  class?: string;
  onChange: (value: number) => void;
}) {
  let dragging = false;
  let pointerId: number | undefined;
  let origin = 0;
  let initialValue = 0;

  const direction = () => props.direction ?? 1;
  const coordinate = (event: PointerEvent) =>
    props.orientation === "vertical" ? event.clientX : event.clientY;

  const beginDrag: JSX.EventHandlerUnion<HTMLDivElement, PointerEvent> = (event) => {
    if (event.button !== 0) return;
    dragging = true;
    pointerId = event.pointerId;
    origin = coordinate(event);
    initialValue = props.value;
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  };

  const moveDrag: JSX.EventHandlerUnion<HTMLDivElement, PointerEvent> = (event) => {
    if (!dragging || event.pointerId !== pointerId) return;
    const delta = (coordinate(event) - origin) * direction();
    props.onChange(clamp(initialValue + delta, props.minimum, props.maximum));
  };

  const endDrag: JSX.EventHandlerUnion<HTMLDivElement, PointerEvent> = (event) => {
    if (event.pointerId !== pointerId) return;
    dragging = false;
    pointerId = undefined;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const handleKeyDown: JSX.EventHandlerUnion<HTMLDivElement, KeyboardEvent> = (event) => {
    const coordinateDelta =
      props.orientation === "vertical"
        ? event.key === "ArrowLeft"
          ? -10
          : event.key === "ArrowRight"
            ? 10
            : 0
        : event.key === "ArrowUp"
          ? -10
          : event.key === "ArrowDown"
            ? 10
            : 0;
    if (coordinateDelta === 0) return;
    event.preventDefault();
    props.onChange(
      clamp(props.value + coordinateDelta * direction(), props.minimum, props.maximum),
    );
  };

  return (
    <div
      class={`workbench-resize-handle ${props.class ?? ""}`}
      classList={{
        "workbench-resize-handle-vertical": props.orientation === "vertical",
        "workbench-resize-handle-horizontal": props.orientation === "horizontal",
      }}
      role="separator"
      tabIndex={0}
      aria-label={props.label}
      aria-orientation={props.orientation}
      aria-valuemin={props.minimum}
      aria-valuemax={props.maximum}
      aria-valuenow={props.value}
      onPointerDown={beginDrag}
      onPointerMove={moveDrag}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onDblClick={() => props.onChange(clamp(props.defaultValue, props.minimum, props.maximum))}
      onKeyDown={handleKeyDown}
    />
  );
}
