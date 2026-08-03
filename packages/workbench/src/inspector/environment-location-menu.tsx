import type { CheckoutKind, WorkbenchEnvironmentSnapshot } from "@hachimi/contracts";
import { Button, Check, FloatingPopover, HardDrive, Laptop } from "@hachimi/ui";
import { createSignal } from "solid-js";

export function EnvironmentLocationMenu(props: {
  environment: WorkbenchEnvironmentSnapshot;
  locale: "zh-CN" | "en-US";
  busy: boolean;
  failure: string | undefined;
  onHandoff: (kind: CheckoutKind) => Promise<void>;
}) {
  const [open, setOpen] = createSignal(false);
  const zh = () => props.locale === "zh-CN";
  const currentKind = () => props.environment.checkout.kind;
  const label = () =>
    currentKind() === "local" ? (zh() ? "本地" : "Local") : zh() ? "工作树" : "Worktree";

  async function select(kind: CheckoutKind) {
    if (kind === currentKind()) return;
    try {
      await props.onHandoff(kind);
      setOpen(false);
    } catch {
      // The owning summary renders the stable Handoff error in this popover.
    }
  }

  return (
    <FloatingPopover
      open={open()}
      onOpenChange={setOpen}
      label={zh() ? "切换工作位置" : "Switch work location"}
      placement="bottom-start"
      contentClass="environment-location-popover"
      triggerClass="environment-summary-row environment-location-trigger"
      triggerTestId="workbench-summary-location"
      trigger={
        <>
          <HardDrive size={16} />
          <strong>{label()}</strong>
          <span class="environment-row-tail">›</span>
        </>
      }
    >
      <header>
        <strong>{zh() ? "工作位置" : "Work location"}</strong>
        <small>{zh() ? "此会话固定复用同一工作树" : "This session reuses one worktree"}</small>
      </header>
      <LocationOption
        kind="local"
        current={currentKind() === "local"}
        disabled={props.busy || !props.environment.handoff.canHandoff}
        label={zh() ? "本地" : "Local"}
        description={zh() ? "项目原始目录" : "Original project directory"}
        onSelect={select}
      />
      <LocationOption
        kind="managed_worktree"
        current={currentKind() === "managed_worktree"}
        disabled={props.busy || !props.environment.handoff.canHandoff}
        label={zh() ? "工作树" : "Worktree"}
        description={zh() ? "此会话的受管隔离目录" : "Managed checkout for this session"}
        onSelect={select}
      />
      {props.environment.handoff.blockedReason ? (
        <p class="environment-inline-error">
          {handoffBlockedText(props.environment.handoff.blockedReason, props.locale)}
        </p>
      ) : null}
      {props.failure ? <p class="environment-inline-error">{props.failure}</p> : null}
    </FloatingPopover>
  );
}

function LocationOption(props: {
  kind: CheckoutKind;
  current: boolean;
  disabled: boolean;
  label: string;
  description: string;
  onSelect: (kind: CheckoutKind) => Promise<void>;
}) {
  return (
    <Button
      class="environment-location-option"
      disabled={props.current || props.disabled}
      onClick={() => void props.onSelect(props.kind)}
    >
      <Laptop size={16} />
      <span>
        <strong>{props.label}</strong>
        <small>{props.description}</small>
      </span>
      {props.current ? <Check size={16} /> : null}
    </Button>
  );
}

function handoffBlockedText(reason: string, locale: "zh-CN" | "en-US") {
  const zh = locale === "zh-CN";
  if (reason === "active_run") return zh ? "运行结束后才能切换位置。" : "Wait for the Run to finish.";
  if (reason === "write_lease") return zh
    ? "当前目录仍被写入操作占用。"
    : "The checkout still has an active write lease.";
  return zh ? "当前无法切换位置。" : "The work location cannot be changed right now.";
}
