import {
  commandFailure,
  commands,
  type AvatarCatalogSnapshot,
  type MotionCatalogSnapshot,
} from "@hachimi/contracts";
import { useI18n } from "@hachimi/i18n";
import {
  Button,
  Hand,
  MetricCard,
  Mic2,
  PageHeading,
  Play,
  RangeField,
  SelectField,
  Square,
} from "@hachimi/ui";
import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { MotionLabPetController } from "./motion-lab-pet-control";
import {
  MotionLabRuntime,
  type MotionLabFrame,
  type MotionTransitionDiagnostic,
  type MotionTransitionMatrixCell,
} from "./motion-lab-runtime";
import {
  motionFamilyLabel,
  motionName,
  motionLoopLabel,
  motionRootLabel,
} from "./motion-localization";

export function MotionLabPage() {
  const i18n = useI18n();
  const text = (zh: string, en: string) => (i18n.locale() === "zh-CN" ? zh : en);
  const [avatars, setAvatars] = createSignal<AvatarCatalogSnapshot>();
  const [motions, setMotions] = createSignal<MotionCatalogSnapshot>({
    entries: [],
    transitionProfiles: [],
    bindings: [],
    disabledMotionIds: [],
  });
  const [avatarId, setAvatarId] = createSignal<string>();
  const [motionId, setMotionId] = createSignal<string>();
  const [transitionSourceId, setTransitionSourceId] = createSignal<string>();
  const [transitionDiagnostic, setTransitionDiagnostic] =
    createSignal<MotionTransitionDiagnostic>();
  const [transitionMatrix, setTransitionMatrix] = createSignal<MotionTransitionMatrixCell[]>([]);
  const [matrixProgress, setMatrixProgress] = createSignal({ completed: 0, total: 0 });
  const [matrixRunning, setMatrixRunning] = createSignal(false);
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
  const analyzableMotions = createMemo(() =>
    motions().entries.filter(
      (entry) =>
        entry.analysisStatus === "ready" && !motions().disabledMotionIds.includes(entry.id),
    ),
  );
  const coreMotions = createMemo(() => selectCoreTransitionMotions(analyzableMotions()));

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
      runtime.setCatalog(motionCatalog.entries, motionCatalog.transitionProfiles);
      const nextAvatar = avatarCatalog.currentId ?? avatarCatalog.entries[0]?.id;
      const requestedMotion = sessionStorage.getItem("hachimi.motionLab.motionId");
      sessionStorage.removeItem("hachimi.motionLab.motionId");
      const nextMotion =
        analyzableEntry(motionCatalog, requestedMotion)?.id ??
        motionCatalog.entries.find(
          (entry) =>
            entry.analysisStatus === "ready" &&
            !motionCatalog.disabledMotionIds.includes(entry.id) &&
            entry.family === "idle" &&
            entry.loopMode === "loop" &&
            /openmaiwaifu/i.test(entry.sourceProject),
        )?.id ??
        motionCatalog.entries.find(
          (entry) =>
            entry.analysisStatus === "ready" && !motionCatalog.disabledMotionIds.includes(entry.id),
        )?.id;
      setAvatarId(nextAvatar);
      setMotionId(nextMotion);
      setTransitionSourceId(nextMotion);
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
    await analyzeTransition();
  }

  async function analyzeTransition() {
    const source = motions().entries.find((entry) => entry.id === transitionSourceId());
    const target = motions().entries.find((entry) => entry.id === motionId());
    if (!source || !target || !runtime) return;
    setTransitionDiagnostic(await runtime.analyzeTransition(source, target));
  }

  async function analyzeMatrix() {
    if (!runtime || matrixRunning()) return;
    setMatrixRunning(true);
    setTransitionMatrix([]);
    setMatrixProgress({ completed: 0, total: coreMotions().length * (coreMotions().length - 1) });
    try {
      const cells = await runtime.analyzeTransitionMatrix(coreMotions(), (completed, total) =>
        setMatrixProgress({ completed, total }),
      );
      setTransitionMatrix(cells);
      setFailure(undefined);
    } catch (error) {
      setFailure(commandFailure(error).message);
    } finally {
      setMatrixRunning(false);
    }
  }

  return (
    <div class="motion-lab-page" data-testid="motion-lab-v5">
      <PageHeading
        class="motion-lab-header"
        eyebrow="Avatar Motion Runtime V5"
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
              options={analyzableMotions().map((entry) => ({
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
          <div class="motion-lab-field">
            <SelectField
              label={text("切换源动作", "Transition source")}
              value={transitionSourceId() ?? ""}
              options={analyzableMotions().map((entry) => ({
                value: entry.id,
                label: motionName(entry, i18n.locale()),
              }))}
              onChange={(value) => {
                setTransitionSourceId(value);
                void analyzeTransition().catch((error) =>
                  setFailure(commandFailure(error).message),
                );
              }}
            />
          </div>
          <section
            class="motion-lab-matrix"
            aria-label={text("核心切换矩阵", "Core transition matrix")}
          >
            <div class="motion-lab-matrix-heading">
              <div>
                <h2>{text("核心切换矩阵", "Core transition matrix")}</h2>
                <p>{`${matrixProgress().completed} / ${matrixProgress().total}`}</p>
              </div>
              <Button
                size="small"
                disabled={matrixRunning() || coreMotions().length < 2}
                onClick={() => void analyzeMatrix()}
              >
                {matrixRunning() ? text("分析中", "Analyzing") : text("运行矩阵", "Run matrix")}
              </Button>
            </div>
            <Show when={transitionMatrix().length > 0}>
              <div class="motion-lab-matrix-scroll">
                <table>
                  <thead>
                    <tr>
                      <th>{text("源", "Source")}</th>
                      <th>{text("目标", "Target")}</th>
                      <th>{text("入口", "Entry")}</th>
                      <th>{text("接触", "Contact")}</th>
                      <th>{text("峰值", "Peak")}</th>
                      <th>{text("准入", "Gate")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={transitionMatrix()}>
                      {(cell) => (
                        <tr classList={{ failed: !cell.accepted }}>
                          <td>{shortMotionName(cell.sourceMotionId)}</td>
                          <td>{shortMotionName(cell.targetMotionId)}</td>
                          <td>{`${cell.entryTimeMs.toFixed(0)} ms`}</td>
                          <td>{`${cell.sourceFootContact}→${cell.targetFootContact}`}</td>
                          <td>{`${cell.peakBoneStepDegrees.toFixed(1)}° / ${(cell.peakRootStepNormalized * 100).toFixed(2)}%`}</td>
                          <td>{cell.accepted ? text("通过", "Pass") : text("失败", "Fail")}</td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </Show>
          </section>
          <section class="motion-lab-pet-test" aria-label={text("桌宠联调", "Pet integration")}>
            <h2>{text("桌宠联调", "Pet integration")}</h2>
            <p>
              {text(
                "通过正式运行时事件在桌宠窗口播放所选动作并测试互动反馈。",
                "Exercise the selected motion and interaction feedback through the real Pet runtime event boundary.",
              )}
            </p>
            <div class="motion-lab-pet-actions">
              <Button
                data-testid="motion-lab-play-pet"
                disabled={!motionId()}
                onClick={() => {
                  const selectedId = motionId() ?? "";
                  const selectedSlot = selectedMotion()?.slot ?? "action";
                  void runPetTest(
                    () => petController.playMotion(selectedId, false, selectedSlot),
                    text("桌宠正在播放所选动作", "Selected motion is playing in Pet"),
                  );
                }}
              >
                <Play size={15} />
                {text("在桌宠播放", "Play in Pet")}
              </Button>
              <Button
                data-testid="motion-lab-stop-pet"
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
                data-testid="motion-lab-touch-head"
                onClick={() =>
                  void runPetTest(
                    () => petController.previewInteraction("head_top"),
                    text("已触发头顶反馈", "Head interaction triggered"),
                  )
                }
              >
                <Hand size={15} />
                {text("头顶反馈", "Head feedback")}
              </Button>
              <Button
                data-testid="motion-lab-speech-start"
                onClick={() =>
                  void runPetTest(
                    () => petController.startSpeech(),
                    text("语音动作已开始", "Speech motion started"),
                  )
                }
              >
                <Mic2 size={15} />
                {text("开始语音", "Start speech")}
              </Button>
              <Button
                data-testid="motion-lab-speech-stop"
                onClick={() =>
                  void runPetTest(
                    () => petController.stopSpeech(),
                    text("语音动作正在释放", "Speech motion is releasing"),
                  )
                }
              >
                <Square size={15} />
                {text("停止语音", "Stop speech")}
              </Button>
            </div>
            <Show when={petTestStatus()}>
              {(status) => <output data-testid="motion-lab-pet-status">{status()}</output>}
            </Show>
          </section>
          <Show when={selectedMotion()}>
            {(entry) => (
              <dl class="motion-lab-diagnostics">
                <dt>{text("来源", "Source")}</dt>
                <dd>{entry().sourceProject}</dd>
                <dt>{text("分类", "Category")}</dt>
                <dd>{motionFamilyLabel(entry().family, i18n.locale())}</dd>
                <dt>Slot / Profile</dt>
                <dd>{`${entry().slot} / ${entry().transitionProfileId}`}</dd>
                <dt>{text("播放", "Playback")}</dt>
                <dd>{motionLoopLabel(entry().loopMode, i18n.locale())}</dd>
                <dt>Root</dt>
                <dd>{motionRootLabel(entry().rootMode, i18n.locale())}</dd>
                <dt>{text("通道", "Channels")}</dt>
                <dd>{entry().channelMask.join(", ")}</dd>
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
                  {entry().loopMode === "loop"
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
                <dt>{text("目标入口", "Target entry")}</dt>
                <dd>{`${(transitionDiagnostic()?.entryTimeMs ?? 0).toFixed(1)} ms`}</dd>
                <dt>{text("切换代价", "Transition cost")}</dt>
                <dd>
                  {transitionDiagnostic()
                    ? `${transitionDiagnostic()!.totalCost.toFixed(4)} · P ${transitionDiagnostic()!.poseCost.toFixed(3)} · V ${transitionDiagnostic()!.velocityCost.toFixed(3)} · F ${transitionDiagnostic()!.footContactCost.toFixed(3)} · R ${transitionDiagnostic()!.rootCost.toFixed(3)}`
                    : "—"}
                </dd>
                <dt>{text("切换方式", "Transition mode")}</dt>
                <dd>
                  {transitionDiagnostic()
                    ? `${transitionDiagnostic()!.durationMs} ms · ${transitionDiagnostic()!.forced ? text("强制", "forced") : text("安全入口", "safe")}`
                    : "—"}
                </dd>
                <dt>{text("源/目标接触", "Source/target contact")}</dt>
                <dd>
                  {transitionDiagnostic()
                    ? `${transitionDiagnostic()!.sourceFootContact} → ${transitionDiagnostic()!.targetFootContact}`
                    : "—"}
                </dd>
                <dt>{text("姿势覆盖", "Pose coverage")}</dt>
                <dd>
                  {transitionDiagnostic()
                    ? `${transitionDiagnostic()!.sourceBoneCount} → ${transitionDiagnostic()!.targetBoneCount}`
                    : "—"}
                </dd>
                <dt>{text("单帧骨骼峰值", "Peak bone step")}</dt>
                <dd>
                  {transitionDiagnostic()
                    ? `${transitionDiagnostic()!.peakBoneStepDegrees.toFixed(2)}° / 12°`
                    : "—"}
                </dd>
                <dt>{text("单帧 Root 峰值", "Peak root step")}</dt>
                <dd>
                  {transitionDiagnostic()
                    ? `${(transitionDiagnostic()!.peakRootStepNormalized * 100).toFixed(3)}% / 0.5% H`
                    : "—"}
                </dd>
                <dt>{text("LookAt 峰值", "Peak LookAt step")}</dt>
                <dd>
                  {transitionDiagnostic()
                    ? `${transitionDiagnostic()!.peakLookAtStepDegrees.toFixed(2)}° / 4°`
                    : "—"}
                </dd>
                <dt>{text("切换准入", "Transition gate")}</dt>
                <dd
                  classList={{ "motion-gate-failed": transitionDiagnostic()?.accepted === false }}
                >
                  {transitionDiagnostic()
                    ? transitionDiagnostic()!.accepted
                      ? text("通过", "Pass")
                      : text("失败", "Fail")
                    : "—"}
                </dd>
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

function analyzableEntry(catalog: MotionCatalogSnapshot, id: string | null) {
  return catalog.entries.find(
    (entry) =>
      entry.id === id &&
      entry.analysisStatus === "ready" &&
      !catalog.disabledMotionIds.includes(entry.id),
  );
}

function selectCoreTransitionMotions(entries: MotionCatalogSnapshot["entries"]) {
  const selected = new Map<string, (typeof entries)[number]>();
  const add = (entry: (typeof entries)[number] | undefined) => {
    if (entry && selected.size < 8) selected.set(entry.id, entry);
  };
  add(entries.find((entry) => entry.family === "idle" && entry.loopMode === "loop"));
  add(entries.find((entry) => entry.family === "reaction"));
  add(entries.find((entry) => entry.family === "speech"));
  for (const role of ["action_recover_to_idle"] as const) {
    add(entries.find((entry) => entry.motionRole === role));
  }
  return [...selected.values()];
}

function shortMotionName(id: string): string {
  return id.replace(/^builtin\./, "").slice(0, 18);
}
