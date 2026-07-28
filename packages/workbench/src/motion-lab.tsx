import {
  commandFailure,
  commands,
  type AvatarCatalogSnapshot,
  type MotionCatalogSnapshot,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  Button,
  MetricCard,
  PageHeading,
  Play,
  RangeField,
  SelectField,
  Square,
} from "@hachimi/ui";
import { Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { MotionLabPetController } from "./motion-lab-pet-control";
import { MotionLabRuntime, type MotionLabFrame } from "./motion-lab-runtime";
import {
  motionCategoryLabel,
  motionName,
  motionPlaybackLabel,
  motionRootLabel,
} from "./motion-localization";

export function MotionLabPage() {
  const i18n = useI18n();
  const text = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const [avatars, setAvatars] = createSignal<AvatarCatalogSnapshot>();
  const [motions, setMotions] = createSignal<MotionCatalogSnapshot>({
    entries: [],
    bindings: [],
    disabledMotionIds: [],
  });
  const [avatarId, setAvatarId] = createSignal<string>();
  const [motionId, setMotionId] = createSignal<string>();
  const [playing, setPlaying] = createSignal(true);
  const [speed, setSpeed] = createSignal(1);
  const [frame, setFrame] = createSignal<MotionLabFrame>();
  const [failure, setFailure] = createSignal<string>();
  const [petTestStatus, setPetTestStatus] = createSignal<string>();
  let stage: HTMLDivElement | undefined;
  let runtime: MotionLabRuntime | undefined;
  const petController = new MotionLabPetController();
  const selectedMotion = createMemo(() =>
    motions().entries.find((entry) => entry.id === motionId()),
  );

  onMount(async () => {
    if (!stage) return;
    runtime = new MotionLabRuntime(stage, { visualMode: "diagnostics" });
    runtime.setFrameListener(setFrame);
    try {
      const [avatarCatalog, motionCatalog] = await Promise.all([
        commands.listAvatarModels(),
        commands.listMotionCatalog(),
      ]);
      setAvatars(avatarCatalog);
      setMotions(motionCatalog);
      runtime.setCatalog(motionCatalog.entries);
      const nextAvatar = avatarCatalog.currentId ?? avatarCatalog.entries[0]?.id;
      const requestedMotion = sessionStorage.getItem("hachimi.motionLab.motionId");
      sessionStorage.removeItem("hachimi.motionLab.motionId");
      const nextMotion =
        motionCatalog.entries.find((entry) => entry.id === requestedMotion)?.id ??
        motionCatalog.entries.find(
          (entry) =>
            entry.category === "idle" &&
            entry.playbackMode === "loop" &&
            /openmaiwaifu/i.test(entry.sourceProject),
        )?.id ??
        motionCatalog.entries[0]?.id;
      setAvatarId(nextAvatar);
      setMotionId(nextMotion);
      if (nextAvatar) await loadAvatar(nextAvatar);
      if (nextMotion) await chooseMotion(nextMotion);
    } catch (error) {
      setFailure(commandFailure(error).message);
    }
  });
  onCleanup(() => {
    runtime?.dispose();
    void petController.stopMotion();
    void petController.stopWalking();
  });

  async function runPetTest(action: () => Promise<void>, success: string) {
    try {
      await action();
      setFailure(undefined);
      setPetTestStatus(success);
    } catch (error) {
      setFailure(commandFailure(error).message);
    }
  }

  async function loadAvatar(id: string) {
    const asset = await commands.getAvatarRuntimeAsset(id);
    if (!asset) throw new Error(text("模型不可用", "Avatar is unavailable"));
    await runtime?.loadAvatar(asset);
  }

  async function chooseMotion(id: string) {
    setMotionId(id);
    const entry = motions().entries.find((value) => value.id === id);
    if (entry) await runtime?.setMotion(entry);
  }

  return (
    <div class="motion-lab-page">
      <PageHeading
        class="motion-lab-header"
        eyebrow="Avatar Motion Runtime V4"
        title={text("动作库实验室", "Motion Library Lab")}
        description={text(
          "直接预览内置和用户 VRMA，检查完整骨骼、手指、Root Motion、循环接缝与约束结果。",
          "Preview built-in and user VRMA assets with full-bone, finger, root-motion, loop, and constraint diagnostics.",
        )}
        badge={`${motions().entries.length} VRMA`}
      />
      <Show when={failure()}>{(message) => <p class="settings-error">{message()}</p>}</Show>
      <div class="motion-lab-layout">
        <section class="motion-lab-stage-card">
          <div class="motion-lab-toolbar">
            <SelectField
              label={text("测试模型", "QA avatar")}
              value={avatarId() ?? ""}
              options={(avatars()?.entries ?? []).map((entry) => ({
                value: entry.id,
                label: entry.name,
              }))}
              onChange={(value) => {
                setAvatarId(value);
                void loadAvatar(value).catch((error) => setFailure(commandFailure(error).message));
              }}
            />
            <Button
              onClick={() => {
                const next = !playing();
                setPlaying(next);
                runtime?.setPlaying(next);
              }}
            >
              {playing() ? <Square size={16} /> : <Play size={16} />}
              {playing() ? text("暂停", "Pause") : text("播放", "Play")}
            </Button>
            <Button onClick={() => runtime?.restart()}>{text("重新开始", "Restart")}</Button>
          </div>
          <div class="motion-lab-stage" ref={stage} />
          <div class="motion-lab-scrubber">
            <RangeField
              label={text("动作时间", "Motion time")}
              min={0}
              max={selectedMotion()?.durationMs ?? 1}
              step={10}
              unit=" ms"
              value={Math.min(frame()?.timeMs ?? 0, selectedMotion()?.durationMs ?? 1)}
              onInput={(value) => runtime?.setTimeMs(value)}
            />
          </div>
          <div class="motion-lab-metrics">
            <Metric label={text("相位", "Phase")} value={(frame()?.phase ?? 0).toFixed(3)} />
            <Metric
              label={text("活动骨骼", "Active bones")}
              value={String(frame()?.activeBones ?? 0)}
            />
            <Metric label={text("手指", "Fingers")} value={String(frame()?.fingerBones ?? 0)} />
            <Metric
              label={text("最大角度", "Max angle")}
              value={`${(frame()?.maxAngleDegrees ?? 0).toFixed(1)}°`}
            />
            <Metric
              label={text("编译缓存", "Compiled cache")}
              value={`${frame()?.compiledCacheSize ?? 0} / 24`}
            />
            <Metric label="Solve" value={`${(frame()?.solveTimeMs ?? 0).toFixed(2)} ms`} />
          </div>
        </section>
        <aside class="motion-lab-panel">
          <div class="motion-lab-field">
            <SelectField
              label={text("VRMA 动作", "VRMA motion")}
              value={motionId() ?? ""}
              options={motions().entries.map((entry) => ({
                value: entry.id,
                label: `${motionName(entry, i18n.locale())} · ${entry.sourceProject}`,
              }))}
              onChange={(value) => void chooseMotion(value)}
            />
          </div>
          <div class="motion-lab-range">
            <RangeField
              label={text("播放速度", "Playback rate")}
              min={0.25}
              max={3}
              step={0.05}
              value={speed()}
              onInput={(value) => {
                setSpeed(value);
                runtime?.setSpeed(value);
              }}
            />
          </div>
          <section class="motion-lab-pet-test" aria-label={text("桌宠联调", "Pet integration")}>
            <h2>{text("桌宠联调", "Pet integration")}</h2>
            <p>
              {text(
                "通过正式运行时事件在桌宠窗口播放所选动作，或测试舞台行走状态机。",
                "Exercise the selected motion and stage locomotion through the real Pet runtime event boundary.",
              )}
            </p>
            <div class="motion-lab-pet-actions">
              <Button
                disabled={!motionId()}
                onClick={() => {
                  const selectedId = motionId() ?? "";
                  void runPetTest(
                    () => petController.playMotion(selectedId),
                    text("桌宠正在播放所选动作", "Selected motion is playing in Pet"),
                  );
                }}
              >
                <Play size={15} />
                {text("在桌宠播放", "Play in Pet")}
              </Button>
              <Button
                onClick={() =>
                  void runPetTest(
                    () => petController.stopMotion(),
                    text("桌宠动作已停止", "Pet motion stopped"),
                  )
                }
              >
                <Square size={15} />
                {text("停止动作", "Stop motion")}
              </Button>
              <Button
                onClick={() =>
                  void runPetTest(
                    () => petController.walkTo(-0.27),
                    text("桌宠正在向左行走", "Pet is walking left"),
                  )
                }
              >
                {text("向左走", "Walk left")}
              </Button>
              <Button
                onClick={() =>
                  void runPetTest(
                    () => petController.walkTo(0.27),
                    text("桌宠正在向右行走", "Pet is walking right"),
                  )
                }
              >
                {text("向右走", "Walk right")}
              </Button>
              <Button
                onClick={() =>
                  void runPetTest(
                    () => petController.stopWalking(),
                    text("桌宠行走已停止", "Pet locomotion stopped"),
                  )
                }
              >
                {text("停止行走", "Stop walking")}
              </Button>
            </div>
            <Show when={petTestStatus()}>{(status) => <output>{status()}</output>}</Show>
          </section>
          <Show when={selectedMotion()}>
            {(entry) => (
              <dl class="motion-lab-diagnostics">
                <dt>{text("来源", "Source")}</dt>
                <dd>{entry().sourceProject}</dd>
                <dt>{text("分类", "Category")}</dt>
                <dd>{motionCategoryLabel(entry().category, i18n.locale())}</dd>
                <dt>{text("播放", "Playback")}</dt>
                <dd>{motionPlaybackLabel(entry().playbackMode, i18n.locale())}</dd>
                <dt>Root</dt>
                <dd>{motionRootLabel(entry().rootMode, i18n.locale())}</dd>
                <dt>{text("通道", "Channels")}</dt>
                <dd>{entry().channels.join(", ")}</dd>
                <dt>{text("骨骼覆盖", "Bone coverage")}</dt>
                <dd>
                  <details>
                    <summary>{entry().animatedBones.length}</summary>
                    <code>{frame()?.activeBoneNames.join(", ") || "—"}</code>
                  </details>
                </dd>
                <dt>{text("手指轨道", "Finger tracks")}</dt>
                <dd>{entry().fingerBoneCount}</dd>
                <dt>Expression</dt>
                <dd>{entry().hasExpression ? "yes" : "no"}</dd>
                <dt>LookAt</dt>
                <dd>{entry().hasLookAt ? "yes" : "no"}</dd>
                <dt>{text("循环接缝", "Loop seam")}</dt>
                <dd>
                  {entry().playbackMode === "loop"
                    ? `${(frame()?.loopSeamDegrees ?? 0).toFixed(2)}° · ${(
                        frame()?.loopSeamRootDistance ?? 0
                      ).toFixed(4)} m`
                    : "—"}
                </dd>
                <dt>{text("Root 轨迹", "Root trajectory")}</dt>
                <dd>{`${(frame()?.rootDistance ?? 0).toFixed(4)} m · [${(
                  frame()?.rootPosition ?? [0, 0, 0]
                )
                  .map((value) => value.toFixed(3))
                  .join(", ")}]`}</dd>
                <dt>{text("足部接触", "Foot contacts")}</dt>
                <dd>{`${frame()?.leftFootPhase ?? "air"} / ${frame()?.rightFootPhase ?? "air"}`}</dd>
                <dt>{text("接触时间线", "Contact timeline")}</dt>
                <dd>
                  <code>{frame()?.contactTimeline || "—"}</code>
                </dd>
                <dt>{text("足底漂移", "Foot drift")}</dt>
                <dd>{`${((frame()?.maxFootDriftNormalized ?? 0) * 100).toFixed(3)}% H`}</dd>
                <dt>{text("地面穿透", "Ground penetration")}</dt>
                <dd>{`${((frame()?.groundPenetrationNormalized ?? 0) * 100).toFixed(3)}% H`}</dd>
                <dt>{text("碰撞", "Collisions")}</dt>
                <dd>{frame()?.collisionCount ?? 0}</dd>
              </dl>
            )}
          </Show>
        </aside>
      </div>
    </div>
  );
}

function Metric(props: { label: string; value: string }) {
  return <MetricCard label={props.label} value={props.value} />;
}
