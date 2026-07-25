import {
  commandFailure,
  commands,
  type AvatarCatalogSnapshot,
  type MotionCatalogEntry,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import { Button, Play, Square, Switch as Toggle } from "@hachimi/ui";
import { Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { MotionLabRuntime, type MotionLabFrame } from "./motion-lab-runtime";

export function MotionPreviewCanvas(props: {
  avatars: AvatarCatalogSnapshot;
  entries: readonly MotionCatalogEntry[];
  motionId: string | undefined;
  mirror: boolean;
  onMirrorChange: (value: boolean) => void;
}) {
  const i18n = useI18n();
  const text = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const [playing, setPlaying] = createSignal(true);
  const [speed, setSpeed] = createSignal(1);
  const [frame, setFrame] = createSignal<MotionLabFrame>();
  const [failure, setFailure] = createSignal<string>();
  let host: HTMLDivElement | undefined;
  let runtime: MotionLabRuntime | undefined;
  let disposed = false;
  let avatarGeneration = 0;
  let motionGeneration = 0;
  const selectedMotion = createMemo(() =>
    props.entries.find((entry) => entry.id === props.motionId),
  );
  const currentAvatar = createMemo(() =>
    props.avatars.entries.find((entry) => entry.id === props.avatars.currentId),
  );

  async function loadAvatar(id: string | null) {
    const generation = ++avatarGeneration;
    if (!id) {
      setFailure(text("当前没有可用模型", "No current avatar is available"));
      return;
    }
    try {
      const asset = await commands.getAvatarRuntimeAsset(id);
      if (!asset) throw new Error(text("模型不可用", "Avatar is unavailable"));
      await runtime?.loadAvatar(asset);
      if (!disposed && generation === avatarGeneration) setFailure(undefined);
    } catch (error) {
      if (!disposed && generation === avatarGeneration) {
        setFailure(commandFailure(error).message);
      }
    }
  }

  async function loadMotion(entry: MotionCatalogEntry | undefined) {
    const generation = ++motionGeneration;
    if (!entry) {
      runtime?.clearMotion();
      return;
    }
    try {
      await runtime?.setMotion(entry);
      if (!disposed && generation === motionGeneration) setFailure(undefined);
    } catch (error) {
      if (!disposed && generation === motionGeneration) {
        setFailure(commandFailure(error).message);
      }
    }
  }

  onMount(() => {
    if (!host) return;
    runtime = new MotionLabRuntime(host, { visualMode: "preview" });
    runtime.setFrameListener(setFrame);
    runtime.setCatalog(props.entries);
    runtime.setMirror(props.mirror);
    void loadAvatar(props.avatars.currentId);
    void loadMotion(selectedMotion());
  });

  createEffect(
    on(
      () => props.avatars.currentId,
      (avatarId) => void loadAvatar(avatarId),
      { defer: true },
    ),
  );
  createEffect(
    on(
      () => props.entries,
      (entries) => runtime?.setCatalog(entries),
      { defer: true },
    ),
  );
  createEffect(
    on(
      () => props.motionId,
      () => void loadMotion(selectedMotion()),
      { defer: true },
    ),
  );
  createEffect(
    on(
      () => props.mirror,
      (mirror) => runtime?.setMirror(mirror),
      { defer: true },
    ),
  );

  onCleanup(() => {
    disposed = true;
    runtime?.dispose();
  });

  return (
    <section class="motion-preview-card" aria-label={text("动作预览", "Motion preview")}>
      <div class="motion-preview-toolbar">
        <div class="motion-preview-current-avatar">
          <span>{text("当前模型", "Current avatar")}</span>
          <strong>{currentAvatar()?.name ?? text("未选择", "Not selected")}</strong>
        </div>
        <Button
          size="small"
          onClick={() => {
            const next = !playing();
            setPlaying(next);
            runtime?.setPlaying(next);
          }}
        >
          {playing() ? <Square size={14} /> : <Play size={14} />}
          {playing() ? text("暂停", "Pause") : text("播放", "Play")}
        </Button>
        <Button size="small" onClick={() => runtime?.restart()}>
          {text("重播", "Restart")}
        </Button>
        <div class="motion-preview-mirror">
          <div>
            <strong>{text("镜像预览", "Mirror preview")}</strong>
            <span>
              {text(
                "仅左右翻转当前预览，不修改 VRMA 文件或互动绑定。",
                "Only flips this preview; the VRMA file and interaction binding are unchanged.",
              )}
            </span>
          </div>
          <Toggle
            checked={props.mirror}
            label={text("镜像预览", "Mirror preview")}
            onChange={props.onMirrorChange}
          />
        </div>
      </div>
      <div class="motion-preview-stage" ref={host} />
      <div class="motion-preview-controls">
        <label>
          <span>{text("时间", "Time")}</span>
          <input
            type="range"
            min={0}
            max={selectedMotion()?.durationMs ?? 1}
            step={10}
            value={Math.min(frame()?.timeMs ?? 0, selectedMotion()?.durationMs ?? 1)}
            onInput={(event) => runtime?.setTimeMs(event.currentTarget.valueAsNumber)}
          />
          <output>{Math.round(frame()?.timeMs ?? 0)} ms</output>
        </label>
        <label>
          <span>{text("速度", "Speed")}</span>
          <input
            type="range"
            min={0.25}
            max={3}
            step={0.05}
            value={speed()}
            onInput={(event) => {
              setSpeed(event.currentTarget.valueAsNumber);
              runtime?.setSpeed(event.currentTarget.valueAsNumber);
            }}
          />
          <output>{speed().toFixed(2)}×</output>
        </label>
      </div>
      <Show when={failure()}>{(message) => <p class="settings-error">{message()}</p>}</Show>
    </section>
  );
}
