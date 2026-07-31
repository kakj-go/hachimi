import { listen } from "@tauri-apps/api/event";
import {
  commandFailure,
  commands,
  type AppSettings,
  type ApprovalRequestRecord,
  type BootstrapState,
  type ClipMotionRequest,
  type InteractiveRegionsUpdate,
  type InteractionMotionPreviewRequest,
  type MotionCatalogSnapshot,
  type PetTurnEvent,
  type PermissionProfile,
  type RuntimeControllerRequest,
  type SpeechPlaybackEvent,
  type SpeechTurnEvent,
  type UserInputRequestRecord,
  type VoiceRuntimeState,
} from "@hachimi/contracts";
import { I18nProvider, useI18n, type AppLocale } from "@hachimi/i18n";
import {
  FloatingIconButton,
  MessageCircle,
  Mic2,
  Send,
  ShieldCheck,
  Square,
  AppearanceProvider,
  Tooltip,
  Volume2,
  VolumeX,
  useTheme,
  type ThemeMode,
} from "@hachimi/ui";
import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { AvatarRuntime } from "./avatar-runtime";
import { collectPetInteractiveRects, exceedsDragThreshold } from "./regions";
import "./pet.css";

type TurnPhase = "idle" | "requesting" | "streaming" | "speaking";

interface PetWindowMotionEvent {
  x: number;
  y: number;
  velocityX: number;
  velocityY: number;
}

function FallbackPet() {
  return (
    <svg class="pet-fallback" viewBox="0 0 238 326" aria-hidden="true">
      <defs>
        <linearGradient id="pet-body" x1="0" y1="0" x2="1" y2="1">
          <stop stop-color="#d8d3ff" />
          <stop offset="1" stop-color="#806fe8" />
        </linearGradient>
      </defs>
      <path
        fill="url(#pet-body)"
        d="M59 116 39 25l72 48c14-3 29-3 43 0l49-48-12 91c24 24 38 60 38 102 0 66-45 102-110 102S9 284 9 218c0-42 19-78 50-102Z"
      />
      <path
        fill="#fff"
        opacity=".72"
        d="M54 229c18 24 42 36 65 36 24 0 48-12 66-36-4 48-27 76-66 76-38 0-61-28-65-76Z"
      />
      <circle cx="80" cy="169" r="13" fill="#34393f" />
      <circle cx="158" cy="169" r="13" fill="#34393f" />
      <circle cx="76" cy="165" r="4" fill="#fff" />
      <circle cx="154" cy="165" r="4" fill="#fff" />
      <path fill="#596270" d="m119 193 13 9-13 11-13-11 13-9Z" />
      <path
        fill="none"
        stroke="#596270"
        stroke-linecap="round"
        stroke-width="5"
        d="M119 211c-7 14-20 17-31 10m31-10c7 14 20 17 31 10"
      />
    </svg>
  );
}

function DesktopPet() {
  const i18n = useI18n();
  const theme = useTheme();
  const [actionsHovered, setActionsHovered] = createSignal(false);
  const [composerOpen, setComposerOpen] = createSignal(false);
  const [draft, setDraft] = createSignal("");
  const [reply, setReply] = createSignal<string>();
  const [voiceNotice, setVoiceNotice] = createSignal<string>();
  const [turnPhase, setTurnPhase] = createSignal<TurnPhase>("idle");
  const [activeRunId, setActiveRunId] = createSignal<string>();
  const [agentRunId, setAgentRunId] = createSignal<string>();
  const [pendingApproval, setPendingApproval] = createSignal<ApprovalRequestRecord>();
  const [pendingUserInput, setPendingUserInput] = createSignal<UserInputRequestRecord>();
  const [inputAnswers, setInputAnswers] = createSignal<Record<string, string>>({});
  const [permissionProfile, setPermissionProfile] = createSignal<PermissionProfile>("read_only");
  const [permissionSessionId, setPermissionSessionId] = createSignal<string>();
  const [muted, setMutedValue] = createSignal(false);
  const [avatarReady, setAvatarReady] = createSignal(false);
  const [avatarLoading, setAvatarLoading] = createSignal(false);
  const [recognizingSpeech, setRecognizingSpeech] = createSignal(false);

  let avatarHitArea: HTMLDivElement | undefined;
  let avatarCanvasHost: HTMLDivElement | undefined;
  let actionBar: HTMLDivElement | undefined;
  let composer: HTMLFormElement | undefined;
  let composerInput: HTMLTextAreaElement | undefined;
  let attentionPanel: HTMLDivElement | undefined;
  let avatarRenderer: AvatarRuntime | undefined;
  let pointerStart:
    | {
        id: number;
        x: number;
        y: number;
        startedAt: number;
        lastX: number;
        lastY: number;
        lastAt: number;
        headPat: boolean;
      }
    | undefined;
  let revision = 0;
  let scheduled = false;
  let hideActionsTimer: number | undefined;
  let pendingPollTimer: number | undefined;
  let speechRevealRunId: string | undefined;
  let speechRevealText = "";
  let speechCommittedText = "";
  let speechFinalText = "";
  let speechSegmentIndex = -1;
  const speechCompletedSegments = new Set<number>();
  const unlisteners: Array<() => void> = [];
  const observer = new ResizeObserver(() => scheduleRegionReport());
  const actionsVisible = () => !composerOpen() && actionsHovered();
  const needsAttention = createMemo(() => pendingApproval() ?? pendingUserInput());

  async function reportRegions() {
    scheduled = false;
    const update: InteractiveRegionsUpdate = {
      windowLabel: "pet",
      revision: ++revision,
      windowWidth: window.innerWidth,
      windowHeight: window.innerHeight,
      regions: collectPetInteractiveRects(
        {
          silhouette: avatarHitArea,
          actionBar,
          composer: composerOpen() ? composer : undefined,
          menuContent: attentionPanel,
          actionsVisible: actionsVisible(),
          menuOpen: Boolean(needsAttention()),
        },
        window.innerWidth,
        window.innerHeight,
      ),
    };
    await commands.setInteractiveRegions(update).catch(() => undefined);
  }

  function scheduleRegionReport() {
    if (scheduled) return;
    scheduled = true;
    // eslint-disable-next-line solid/reactivity -- animation-frame callbacks read the latest interaction signals.
    window.requestAnimationFrame(() => void reportRegions());
  }

  function showActions() {
    if (hideActionsTimer !== undefined) window.clearTimeout(hideActionsTimer);
    hideActionsTimer = undefined;
    setActionsHovered(true);
    scheduleRegionReport();
  }

  function hideActionsSoon() {
    if (hideActionsTimer !== undefined) window.clearTimeout(hideActionsTimer);
    hideActionsTimer = window.setTimeout(() => {
      hideActionsTimer = undefined;
      setActionsHovered(false);
      scheduleRegionReport();
    }, 650);
  }

  function openComposer() {
    if (hideActionsTimer !== undefined) window.clearTimeout(hideActionsTimer);
    hideActionsTimer = undefined;
    setActionsHovered(false);
    setComposerOpen(true);
    scheduleRegionReport();
    queueMicrotask(() => composerInput?.focus());
  }

  async function closeComposer(cancel = false) {
    if (cancel && activeRunId()) await commands.cancelPetTurn().catch(() => undefined);
    setComposerOpen(false);
    scheduleRegionReport();
  }

  async function submitTurn() {
    if (activeRunId()) {
      await commands.cancelPetTurn().catch(() => undefined);
      return;
    }
    const text = draft().trim();
    if (!text) return;
    const runId = crypto.randomUUID();
    resetSpeechReveal();
    setActiveRunId(runId);
    setTurnPhase("requesting");
    setReply("");
    setVoiceNotice(undefined);
    setDraft("");
    try {
      await commands.startPetTurn({ runId, text });
    } catch (error) {
      if (activeRunId() === runId) {
        setActiveRunId(undefined);
        setTurnPhase("idle");
        setReply(commandFailure(error).message);
      }
    }
  }

  async function recognizeSpeech() {
    if (recognizingSpeech() || activeRunId()) return;
    setRecognizingSpeech(true);
    try {
      const text = (await commands.recognizePetSpeech()).trim();
      if (text && composerOpen()) {
        setDraft((current) => `${current}${current.trim() ? " " : ""}${text}`);
        queueMicrotask(() => composerInput?.focus());
      }
    } catch (error) {
      setReply(commandFailure(error).message);
    } finally {
      setRecognizingSpeech(false);
      scheduleRegionReport();
    }
  }

  function handleTurnEvent(event: PetTurnEvent) {
    if (event.runId !== activeRunId()) return;
    if (event.type === "started") {
      setPermissionSessionId(event.session_id);
      setAgentRunId(event.agent_run_id);
      setTurnPhase("streaming");
      avatarRenderer?.setListening(true);
    } else if (event.type === "completed") {
      setActiveRunId(undefined);
      setAgentRunId(undefined);
      setPendingApproval(undefined);
      setPendingUserInput(undefined);
      if (event.speechQueued) {
        speechRevealRunId = event.runId;
        speechFinalText = event.text;
        setTurnPhase("speaking");
      } else {
        setReply(event.text);
        setTurnPhase("idle");
        avatarRenderer?.setListening(false);
      }
    } else if (event.type === "cancelled") {
      setTurnPhase("idle");
      setActiveRunId(undefined);
      setAgentRunId(undefined);
      setPendingApproval(undefined);
      setPendingUserInput(undefined);
      avatarRenderer?.setListening(false);
    } else if (event.type === "failed") {
      setReply(event.message);
      setTurnPhase("idle");
      setActiveRunId(undefined);
      setAgentRunId(undefined);
      setPendingApproval(undefined);
      setPendingUserInput(undefined);
      avatarRenderer?.setListening(false);
    }
    scheduleRegionReport();
  }

  async function refreshPendingInteraction() {
    const sessionId = permissionSessionId();
    const runId = agentRunId();
    if (!sessionId || !runId || !activeRunId()) {
      setPendingApproval(undefined);
      setPendingUserInput(undefined);
      return;
    }
    try {
      const [approvals, inputs] = await Promise.all([
        commands.listPendingApprovals(sessionId),
        commands.listPendingUserInput(sessionId),
      ]);
      const approval = approvals.find((record) => record.runId === runId);
      const input = inputs.find((record) => record.runId === runId);
      setPendingApproval(approval);
      setPendingUserInput(input);
      if (input) {
        setInputAnswers((current) => {
          const next = { ...current };
          for (const question of input.questions) {
            next[question.id] ??= question.defaultAnswer ?? question.options[0]?.value ?? "";
          }
          return next;
        });
      }
      if (approval || input) {
        setReply(undefined);
        scheduleRegionReport();
      }
    } catch {
      // A concurrent Workbench resolution is expected; the next poll reconciles it by id.
    }
  }

  async function resolveApproval(decision: "approved" | "denied") {
    const approval = pendingApproval();
    if (!approval) return;
    try {
      await commands.resolveAgentApproval({
        approvalId: approval.id,
        decision,
        expectedRunId: approval.runId,
        expectedGeneration: approval.runGeneration,
      });
    } catch (error) {
      setReply(commandFailure(error).message);
    } finally {
      await refreshPendingInteraction();
    }
  }

  async function resolveInput(action: "submit" | "decline") {
    const input = pendingUserInput();
    if (!input) return;
    try {
      await commands.resolveUserInput({
        requestId: input.id,
        expectedRunId: input.runId,
        expectedGeneration: input.runGeneration,
        action,
        answers:
          action === "submit"
            ? input.questions.map((question) => ({
                questionId: question.id,
                value: inputAnswers()[question.id] ?? "",
              }))
            : [],
        resolvedBy: "",
        resolvedAtMs: Date.now(),
      });
      setInputAnswers({});
    } catch (error) {
      setReply(commandFailure(error).message);
    } finally {
      await refreshPendingInteraction();
    }
  }

  async function togglePermissionProfile() {
    if (activeRunId()) return;
    const next = permissionProfile() === "read_only" ? "external_sandbox" : "read_only";
    try {
      const config = await commands.updateSessionPermissionConfig({
        sessionId: permissionSessionId() ?? null,
        entryProfile: "pet_conversation",
        config: {
          permissionProfile: next,
          approvalPolicy: "only_when_needed",
        },
      });
      setPermissionProfile(config.permissionProfile);
    } catch (error) {
      setReply(commandFailure(error).message);
    }
  }

  function resetSpeechReveal() {
    speechRevealRunId = undefined;
    speechRevealText = "";
    speechCommittedText = "";
    speechFinalText = "";
    speechSegmentIndex = -1;
    speechCompletedSegments.clear();
  }

  function finishSpeechSegment(forceFull: boolean) {
    if (forceFull && !speechCompletedSegments.has(speechSegmentIndex)) {
      speechCommittedText = `${speechCommittedText}${speechRevealText}`;
      speechCompletedSegments.add(speechSegmentIndex);
      setReply(speechCommittedText);
    }
    scheduleRegionReport();
  }

  function finishSpeechTurn() {
    finishSpeechSegment(false);
    if (speechFinalText) setReply(speechFinalText);
    setTurnPhase("idle");
    speechRevealRunId = undefined;
    speechRevealText = "";
    speechCommittedText = "";
    speechFinalText = "";
    speechSegmentIndex = -1;
    speechCompletedSegments.clear();
    avatarRenderer?.setListening(false);
    scheduleRegionReport();
  }

  function handleSpeechPlayback(event: SpeechPlaybackEvent) {
    avatarRenderer?.handleSpeechPlayback(event);
    if (event.source !== "pet_turn" || !event.runId) return;
    if (event.phase === "prepared") {
      if (speechRevealRunId !== event.runId) {
        resetSpeechReveal();
        speechRevealRunId = event.runId;
      }
      speechRevealRunId = event.runId;
      speechRevealText = event.displayText ?? "";
      speechSegmentIndex = event.segmentIndex;
      setTurnPhase("speaking");
      return;
    }
    if (event.runId !== speechRevealRunId) return;
    if (event.phase === "playing") {
      if (!speechCompletedSegments.has(event.segmentIndex)) {
        speechCommittedText = `${speechCommittedText}${speechRevealText}`;
        speechCompletedSegments.add(event.segmentIndex);
        setReply(speechCommittedText);
        scheduleRegionReport();
      }
    } else if (event.phase === "completed" || event.phase === "failed") {
      finishSpeechSegment(true);
    } else if (event.phase === "stopped") {
      finishSpeechSegment(false);
      setTurnPhase("idle");
    }
  }

  function handleSpeechTurn(event: SpeechTurnEvent) {
    if (event.runId !== speechRevealRunId && event.runId !== activeRunId()) return;
    if (event.phase === "completed" || event.phase === "skipped" || event.phase === "failed") {
      finishSpeechTurn();
      if (event.phase === "skipped") {
        setVoiceNotice(i18n.t("pet.voiceChineseOnly"));
      } else if (event.phase === "failed") {
        setVoiceNotice(i18n.t("pet.voiceSynthesisFailed"));
      }
    } else if (event.phase === "stopped") {
      finishSpeechSegment(false);
      setTurnPhase("idle");
    }
  }

  async function refreshAvatar() {
    if (!avatarRenderer) return;
    try {
      const asset = await commands.getCurrentAvatarAsset();
      if (!asset) {
        avatarRenderer.clear();
        setAvatarReady(false);
        return;
      }
      setAvatarLoading(true);
      await avatarRenderer.load(asset);
      setAvatarReady(true);
      setReply(undefined);
    } catch (error) {
      if (!avatarReady()) setReply(commandFailure(error).message);
    } finally {
      setAvatarLoading(false);
      scheduleRegionReport();
    }
  }

  async function setMuted(mute: boolean) {
    try {
      const state = await commands.setMuted(mute);
      setMutedValue(state.muted);
    } catch (error) {
      setReply(commandFailure(error).message);
    }
  }

  function pointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    const now = performance.now();
    pointerStart = {
      id: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      startedAt: now,
      lastX: event.clientX,
      lastY: event.clientY,
      lastAt: now,
      headPat: avatarRenderer?.beginHeadPatAt(event.clientX, event.clientY) ?? false,
    };
    avatarHitArea?.setPointerCapture(event.pointerId);
  }

  function pointerMove(event: PointerEvent) {
    avatarRenderer?.trackCursorAt(event.clientX, event.clientY);
    if (!pointerStart || pointerStart.id !== event.pointerId) return;
    if (pointerStart.headPat) {
      const now = performance.now();
      const elapsed = Math.max(now - pointerStart.lastAt, 1);
      const speed =
        Math.hypot(event.clientX - pointerStart.lastX, event.clientY - pointerStart.lastY) /
        elapsed;
      const retained = avatarRenderer?.updateHeadPatAt(
        event.clientX,
        event.clientY,
        now - pointerStart.startedAt,
        speed,
      );
      if (!retained) {
        avatarRenderer?.endHeadPat();
        pointerStart = undefined;
        return;
      }
      pointerStart.lastX = event.clientX;
      pointerStart.lastY = event.clientY;
      pointerStart.lastAt = now;
      return;
    }
    if (!exceedsDragThreshold({ clientX: pointerStart.x, clientY: pointerStart.y }, event)) return;
    pointerStart = undefined;
    avatarRenderer?.setDragging(true);
    void commands
      .startPetDragging()
      .catch((error) => setReply(commandFailure(error).message))
      .finally(() => avatarRenderer?.setDragging(false));
  }

  function pointerUp(event: PointerEvent) {
    if (!pointerStart || pointerStart.id !== event.pointerId) return;
    if (pointerStart.headPat) {
      avatarRenderer?.endHeadPat();
      pointerStart = undefined;
      return;
    }
    pointerStart = undefined;
    avatarRenderer?.interactAt(event.clientX, event.clientY);
  }

  function pointerCancel(event: PointerEvent) {
    if (!pointerStart || pointerStart.id !== event.pointerId) return;
    if (pointerStart.headPat) avatarRenderer?.endHeadPat();
    pointerStart = undefined;
  }

  function pointerLeave() {
    avatarRenderer?.clearCursorAttention();
    hideActionsSoon();
  }

  onMount(() => {
    if (avatarCanvasHost) {
      try {
        avatarRenderer = new AvatarRuntime(avatarCanvasHost);
      } catch {
        setAvatarReady(false);
      }
    }
    for (const element of [avatarHitArea, actionBar, composer]) {
      if (element) observer.observe(element);
    }
    window.addEventListener("resize", scheduleRegionReport);
    void Promise.all([
      listen<AppSettings>("settings-changed", ({ payload }) => {
        setMutedValue(payload.voice.muted);
        theme.setMode(payload.theme as ThemeMode);
        theme.setAppearance(payload.appearance);
        i18n.setLocale(payload.locale as AppLocale);
      }),
      listen<VoiceRuntimeState>("voice-runtime-changed", ({ payload }) => {
        setMutedValue(payload.muted);
        if (payload.muted) avatarRenderer?.stopSpeech();
      }),
      listen<SpeechPlaybackEvent>("voice:playback", ({ payload }) => {
        handleSpeechPlayback(payload);
      }),
      // eslint-disable-next-line solid/reactivity -- Tauri events intentionally update live signals after mount.
      listen<SpeechTurnEvent>("voice:turn", ({ payload }) => {
        handleSpeechTurn(payload);
      }),
      listen<boolean>("pet:visibility", ({ payload }) => {
        avatarRenderer?.setPaused(!payload);
      }),
      listen<PetWindowMotionEvent>("pet:window-motion", ({ payload }) => {
        avatarRenderer?.updateWindowMotion(payload.velocityX, payload.velocityY);
      }),
      listen<MotionCatalogSnapshot>("motion:catalog-changed", ({ payload }) => {
        avatarRenderer?.setMotionCatalog(payload);
      }),
      listen<ClipMotionRequest>("motion:clip-request", ({ payload }) => {
        avatarRenderer?.playClipMotion(payload);
      }),
      listen<RuntimeControllerRequest>("motion:controller-request", ({ payload }) => {
        avatarRenderer?.applyRuntimeController(payload);
      }),
      listen<InteractionMotionPreviewRequest>("motion:preview-interaction", ({ payload }) => {
        avatarRenderer?.previewInteraction(payload.region);
      }),
      // eslint-disable-next-line solid/reactivity -- Tauri events intentionally update live signals after mount.
      listen<PetTurnEvent>("pet:turn", ({ payload }) => handleTurnEvent(payload)),
      listen("pet:close-composer", () => {
        setComposerOpen(false);
        scheduleRegionReport();
      }),
      listen("pet:open-composer", () => openComposer()),
      // eslint-disable-next-line solid/reactivity -- the refresh event intentionally reads the current renderer.
      listen("pet:refresh-avatar", () => void refreshAvatar()),
    ]).then((values) => unlisteners.push(...values));
    void commands
      .getVoiceRuntimeState()
      .then((state) => setMutedValue(state.muted))
      .catch((error) => setReply(commandFailure(error).message));
    void commands
      .getSessionPermissionConfig({ sessionId: null, entryProfile: "pet_conversation" })
      .then((config) => setPermissionProfile(config.permissionProfile))
      .catch(() => undefined);
    // Load the motion catalog before the avatar so its cool base idle can be
    // prepared while the SVG fallback remains visible. This prevents a rest/T-pose flash.
    void commands
      .listMotionCatalog()
      .then((catalog) => avatarRenderer?.setMotionCatalog(catalog))
      .catch(() => undefined)
      // eslint-disable-next-line solid/reactivity -- initial async bootstrap intentionally refreshes the current avatar after its motion catalog is ready.
      .then(() => refreshAvatar());
    // eslint-disable-next-line solid/reactivity -- the interval intentionally reads the latest pending interaction state.
    pendingPollTimer = window.setInterval(() => void refreshPendingInteraction(), 500);
    void reportRegions().then(() => commands.frontendReady());
  });

  onCleanup(() => {
    unlisteners.forEach((unlisten) => unlisten());
    if (hideActionsTimer !== undefined) window.clearTimeout(hideActionsTimer);
    if (pendingPollTimer !== undefined) window.clearInterval(pendingPollTimer);
    observer.disconnect();
    avatarRenderer?.dispose();
    window.removeEventListener("resize", scheduleRegionReport);
  });

  return (
    <main
      class="pet-stage"
      data-testid="pet-stage"
      data-agent-run-id={agentRunId()}
      data-session-id={permissionSessionId()}
      data-actions-visible={actionsVisible()}
      data-composer-open={composerOpen()}
      aria-label={i18n.t("pet.testLabel")}
    >
      <Show when={needsAttention()}>
        <div
          ref={attentionPanel}
          class="pet-attention"
          data-testid="pet-attention"
          role="dialog"
          aria-live="polite"
        >
          <Show when={reply()}>
            {(message) => <small data-testid="pet-attention-error">{message()}</small>}
          </Show>
          <Show when={pendingApproval()}>
            {(approval) => (
              <>
                <strong>{i18n.locale() === "zh-CN" ? "需要审批" : "Approval required"}</strong>
                <span>{approval().riskSummary}</span>
                <small>
                  {approval().action} · {approval().resource}
                </small>
                <div class="pet-attention-actions">
                  <button
                    type="button"
                    data-testid="pet-deny-approval"
                    onClick={() => void resolveApproval("denied")}
                  >
                    {i18n.locale() === "zh-CN" ? "拒绝" : "Deny"}
                  </button>
                  <button
                    type="button"
                    data-primary="true"
                    data-testid="pet-approve-once"
                    onClick={() => void resolveApproval("approved")}
                  >
                    {i18n.locale() === "zh-CN" ? "仅本次允许" : "Allow once"}
                  </button>
                </div>
              </>
            )}
          </Show>
          <Show when={pendingUserInput()}>
            {(request) => (
              <>
                <strong>{i18n.locale() === "zh-CN" ? "需要你的输入" : "Input required"}</strong>
                <For each={request().questions}>
                  {(question) => (
                    <label class="pet-attention-question">
                      <span>{question.prompt}</span>
                      <Show
                        when={question.options.length > 0}
                        fallback={
                          <input
                            type={question.secret ? "password" : "text"}
                            value={inputAnswers()[question.id] ?? ""}
                            onInput={(event) =>
                              setInputAnswers((current) => ({
                                ...current,
                                [question.id]: event.currentTarget.value,
                              }))
                            }
                          />
                        }
                      >
                        <select
                          value={inputAnswers()[question.id] ?? ""}
                          onChange={(event) =>
                            setInputAnswers((current) => ({
                              ...current,
                              [question.id]: event.currentTarget.value,
                            }))
                          }
                        >
                          <For each={question.options}>
                            {(option) => <option value={option.value}>{option.label}</option>}
                          </For>
                        </select>
                      </Show>
                    </label>
                  )}
                </For>
                <div class="pet-attention-actions">
                  <button
                    type="button"
                    data-testid="pet-decline-user-input"
                    onClick={() => void resolveInput("decline")}
                  >
                    {i18n.locale() === "zh-CN" ? "暂不提供" : "Decline"}
                  </button>
                  <button
                    type="button"
                    data-primary="true"
                    data-testid="pet-submit-user-input"
                    onClick={() => void resolveInput("submit")}
                  >
                    {i18n.locale() === "zh-CN" ? "提交" : "Submit"}
                  </button>
                </div>
              </>
            )}
          </Show>
          <button
            class="pet-attention-workbench"
            type="button"
            data-testid="pet-open-workbench"
            onClick={() => void commands.openWorkbench("home")}
          >
            {i18n.locale() === "zh-CN" ? "在工作台中查看" : "Open in Workbench"}
          </button>
        </div>
      </Show>
      <Show when={reply() && !needsAttention()}>
        <div class="pet-speech" role="status" aria-live="polite">
          {reply()}
          <Show when={turnPhase() !== "idle"}>
            <span class="pet-stream-caret" aria-hidden="true" />
          </Show>
          <Show when={voiceNotice()}>
            {(notice) => <span class="pet-voice-notice">{notice()}</span>}
          </Show>
        </div>
      </Show>
      <div
        ref={avatarHitArea}
        class="pet-avatar-hit-area"
        role="button"
        tabIndex={0}
        aria-label={i18n.t("pet.testLabel")}
        onPointerEnter={showActions}
        onPointerLeave={pointerLeave}
        onPointerDown={pointerDown}
        onPointerMove={pointerMove}
        onPointerUp={pointerUp}
        onPointerCancel={pointerCancel}
        onContextMenu={(event) => {
          event.preventDefault();
          void commands
            .showPetContextMenu({ x: event.clientX, y: event.clientY })
            .catch((error) => setReply(commandFailure(error).message));
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            avatarRenderer?.interact();
          }
        }}
      >
        <div ref={avatarCanvasHost} class="pet-avatar-canvas-host" />
        <Show when={!avatarReady()}>
          <FallbackPet />
        </Show>
        <Show when={avatarLoading()}>
          <span class="pet-model-loader" aria-hidden="true" />
        </Show>
      </div>
      <Show when={composerOpen()}>
        <form
          ref={composer}
          class="pet-composer"
          onSubmit={(event) => {
            event.preventDefault();
            void submitTurn();
          }}
        >
          <textarea
            ref={composerInput}
            data-testid="pet-composer-input"
            value={draft()}
            maxlength={8000}
            rows={2}
            placeholder={i18n.t("pet.inputPlaceholder")}
            aria-label={i18n.t("pet.inputPlaceholder")}
            onInput={(event) => setDraft(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                void submitTurn();
              } else if (event.key === "Escape") {
                event.preventDefault();
                void closeComposer(true);
              }
            }}
          />
          <button
            class="pet-composer-microphone"
            type="button"
            data-state={recognizingSpeech() ? "listening" : "idle"}
            aria-label={
              recognizingSpeech() ? i18n.t("pet.voiceListening") : i18n.t("pet.voiceInput")
            }
            aria-pressed={recognizingSpeech()}
            disabled={Boolean(activeRunId()) || recognizingSpeech()}
            onClick={() => void recognizeSpeech()}
          >
            <Mic2 size={16} />
          </button>
          <button
            class="pet-composer-submit"
            type="submit"
            data-testid="pet-composer-submit"
            aria-label={activeRunId() ? i18n.t("pet.stop") : i18n.t("pet.sendMessage")}
            disabled={!activeRunId() && !draft().trim()}
          >
            <Show when={activeRunId()} fallback={<Send size={16} />}>
              <Square size={14} fill="currentColor" />
            </Show>
          </button>
        </form>
      </Show>
      <div
        ref={actionBar}
        class="pet-actions"
        data-state={actionsVisible() ? "visible" : "hidden"}
        aria-hidden={!actionsVisible()}
        onPointerEnter={showActions}
        onPointerLeave={hideActionsSoon}
      >
        <Tooltip label={i18n.t("pet.sendMessage")}>
          <FloatingIconButton
            class="pet-action"
            size="small"
            data-testid="pet-open-composer"
            label={i18n.t("pet.sendMessage")}
            onClick={openComposer}
          >
            <MessageCircle size={17} />
          </FloatingIconButton>
        </Tooltip>
        <Tooltip
          label={
            permissionProfile() === "read_only"
              ? i18n.locale() === "zh-CN"
                ? "只读权限"
                : "Read-only permissions"
              : i18n.locale() === "zh-CN"
                ? "外部工具权限（仍需审批）"
                : "External tools (approvals still apply)"
          }
        >
          <FloatingIconButton
            class="pet-action"
            size="small"
            data-testid="pet-permission-toggle"
            label="Agent permissions"
            aria-pressed={permissionProfile() === "external_sandbox"}
            disabled={Boolean(activeRunId())}
            onClick={() => void togglePermissionProfile()}
          >
            <ShieldCheck size={17} />
          </FloatingIconButton>
        </Tooltip>
        <Tooltip label={muted() ? i18n.t("pet.unmute") : i18n.t("pet.mute")}>
          <FloatingIconButton
            class="pet-action"
            size="small"
            label={muted() ? i18n.t("pet.unmute") : i18n.t("pet.mute")}
            aria-pressed={muted()}
            onClick={() => void setMuted(!muted())}
          >
            <Show when={muted()} fallback={<Volume2 size={17} />}>
              <VolumeX size={17} />
            </Show>
          </FloatingIconButton>
        </Tooltip>
      </div>
    </main>
  );
}

export function PetApp() {
  const [bootstrap, setBootstrap] = createSignal<BootstrapState>();
  const [failure, setFailure] = createSignal<string>();

  onMount(async () => {
    try {
      setBootstrap(await commands.getBootstrapState());
    } catch (error) {
      setFailure(commandFailure(error).message);
      await commands.frontendReady().catch(() => undefined);
    }
  });

  return (
    <Show when={bootstrap()} fallback={<div class="pet-speech">{failure()}</div>}>
      {(state) => (
        <AppearanceProvider
          initialMode={state().theme as ThemeMode}
          initialAppearance={state().appearance}
        >
          <I18nProvider initialLocale={state().locale as AppLocale}>
            <DesktopPet />
          </I18nProvider>
        </AppearanceProvider>
      )}
    </Show>
  );
}
