import type {
  AvatarAdaptationProfile,
  AvatarRuntimeAsset,
  BehaviorChannel,
  InteractionRegion,
  MotionCatalogEntry,
  MotionCatalogSnapshot,
  MotionIntentRequest,
  MotionTransitionProfile,
  RuntimeControllerRequest,
  SpeechPlaybackEvent,
  SpeechTimeline,
} from "@hachimi/contracts";
import { commands } from "@hachimi/contracts";
import { VRMLoaderPlugin, VRMUtils, type VRM, type VRMHumanBoneName } from "@pixiv/three-vrm";
import { VRMAnimationLoaderPlugin } from "@pixiv/three-vrm-animation";
import {
  AmbientLight,
  Box3,
  Clock,
  Color,
  DirectionalLight,
  Euler,
  Group,
  HemisphereLight,
  MathUtils,
  NeutralToneMapping,
  PerspectiveCamera,
  Quaternion,
  Raycaster,
  Scene,
  SRGBColorSpace,
  SkinnedMesh,
  Vector2,
  Vector3,
  WebGLRenderer,
} from "three";
import type { Intersection, Mesh, Object3D } from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import {
  ambientMotionDelayMs,
  classifyRelativeHit,
  rememberAmbientMotion,
  selectAmbientIdle,
  selectWaitingIdle,
  isPlaybackOlder,
  speechReleaseEnvelope,
} from "../avatar-runtime-logic";
import { FaceExpressionMixer, FaceGazeRuntime, type ExpressionLayer } from "../face-gaze-runtime";
import {
  AvatarConstraintPipeline,
  FootContactAnalyzer,
  MotionAssetLibrary,
  StageLocomotionController,
  applyCanonicalBoneEuler,
  composeMotionLayers,
  AnimationGraph,
  FullPoseInertializer,
  velocityBetweenPoses,
  zeroVelocity,
  deepDisposeAvatarRoot,
  loadAvatarWithDomTextures,
  selectMotionForIntent,
  type FootContactState,
  type FootSoleOffsets,
  type StageLocomotionPhase,
  type SampledMotionPose,
  type MotionFeatureFrame,
} from "@hachimi/avatar-motion-runtime";
import { SecondaryMotionRuntime } from "../secondary-motion-runtime";
import { BehaviorScheduler, PET_MOTION_PRIORITIES } from "./behavior-scheduler";
import { InteractionFeedbackRuntime } from "./interaction-feedback";
import { PetMotionOrchestrator } from "./motion-orchestrator";
import { MotionPreloader } from "./motion-preloader";
import { runAvatarFramePipeline } from "./avatar-frame-pipeline";
import { cloneSampledPose, isMouthExpression, limitPoseStep } from "./motion-continuity";
import {
  capturePresentationRootBaseline,
  contactState,
  finiteNumber,
  isLeftRegion,
  locomotionChannelWeights,
  profileSoleOffsets,
  restorePresentationRoot,
  speechChannelWeights,
  stabilizePresentationRoot,
  type AvatarPointerHit,
  type PresentationRootBaseline,
} from "./avatar-runtime-support";

export {
  capturePresentationRootBaseline,
  restorePresentationRoot,
  stabilizePresentationRoot,
} from "./avatar-runtime-support";

export { applyCanonicalBoneEuler } from "@hachimi/avatar-motion-runtime";

interface SpeechState {
  playbackId: string;
  segmentIndex: number;
  sequence: number;
  durationMs: number;
  mediaPositionMs: number;
  receivedAt: number;
  timeline: SpeechTimeline;
  playing: boolean;
  stoppingAt?: number;
  releaseFrom: number;
}

interface MorphTarget {
  expression: string;
  hosts: Array<{ influences: number[]; index: number }>;
}

interface AvatarInstance {
  asset: AvatarRuntimeAsset;
  presentationRoot: Group;
  rootBaseline: PresentationRootBaseline;
  contentRoot: Group;
  vrm: VRM;
  bounds: Box3;
  height: number;
  bones: Map<string, Object3D>;
  rawBones: Map<string, Object3D>;
  semanticRegions: Map<Object3D, InteractionRegion>;
  restRotations: Map<string, Quaternion>;
  restPositions: Map<string, Vector3>;
  morphTargets: MorphTarget[];
  soleOffsets: FootSoleOffsets;
}

interface RuntimeMotionNode {
  id: string;
  intent: MotionIntentRequest;
  ready: boolean;
}

interface CatalogMotionFrame {
  lookAt?: { yawDegrees: number; pitchDegrees: number };
}

interface CatalogMotionComposition {
  pose: SampledMotionPose;
  signature: string;
  inertialHalfLives?: Parameters<FullPoseInertializer["apply"]>[2];
}

export class AvatarRuntime {
  readonly canvas: HTMLCanvasElement;
  private readonly renderer: WebGLRenderer;
  private readonly scene = new Scene();
  private readonly camera = new PerspectiveCamera(28, 1, 0.01, 10_000);
  private readonly loader = new GLTFLoader();
  private readonly motionLoader = new GLTFLoader();
  private readonly motionLibrary: MotionAssetLibrary;
  private readonly animationGraph: AnimationGraph;
  private readonly resizeObserver: ResizeObserver;
  private readonly clock = new Clock();
  private readonly raycaster = new Raycaster();
  private readonly pointer = new Vector2();
  private readonly rotationScratch = Array.from({ length: 7 }, () => new Quaternion());
  private readonly rotationEuler = new Euler();
  private instance: AvatarInstance | undefined;
  private speech: SpeechState | undefined;
  private latestPlaybackId: string | undefined;
  private revision = 0;
  private animationFrame = 0;
  private disposed = false;
  private paused = false;
  private visibilityPaused = false;
  private dragging = false;
  private dragVelocity = new Vector2();
  private lastInteractionAt = 0;
  private motionCatalog: MotionCatalogSnapshot = {
    entries: [],
    transitionProfiles: [],
    bindings: [],
    disabledMotionIds: [],
  };
  private readonly motionEntries = new Map<string, MotionCatalogEntry>();
  private readonly transitionProfiles = new Map<string, MotionTransitionProfile>();
  private disabledMotionIds = new Set<string>();
  private readonly interactionCooldowns = new Map<InteractionRegion, number>();
  private readonly entrancePlayedForAvatarIds = new Set<string>();
  private idleMotion: RuntimeMotionNode | undefined;
  private entranceMotion: RuntimeMotionNode | undefined;
  private startupSequenceComplete = false;
  private ambientMotion: RuntimeMotionNode | undefined;
  private nextAmbientMotionAt = 0;
  private recentAmbientMotionIds: string[] = [];
  private foregroundWasActive = false;
  private interactionMotion: RuntimeMotionNode | undefined;
  private actionRecoveryMotion: RuntimeMotionNode | undefined;
  private speechMotion: RuntimeMotionNode | undefined;
  private locomotionMotion: RuntimeMotionNode | undefined;
  private readonly requestedMotions = new Map<string, RuntimeMotionNode>();
  private readonly controllerSequences = new Map<RuntimeControllerRequest["kind"], number>();
  private listening = false;
  private headPatActive = false;
  private headPatDirection: -1 | 1 = 1;
  private headPatPressure = 0;
  private interactionCorrection:
    | { startedAt: number; direction: -1 | 1; strength: number }
    | undefined;
  private motionSetRevision = 0;
  private speechGestureCursor = 0;
  private readonly transitionInertializer = new FullPoseInertializer();
  private motionRequestSequence = 0;
  private lastFootContact: FootContactState = "unknown";
  private motionTransitionSignature = "";
  private previousResolvedPose: SampledMotionPose | undefined;
  private resolvedPoseBeforePrevious: SampledMotionPose | undefined;
  private readonly behaviorScheduler = new BehaviorScheduler<{
    region: InteractionRegion;
    direction: -1 | 1;
  }>();
  private readonly motionOrchestrator = new PetMotionOrchestrator();
  private readonly interactionFeedback = new InteractionFeedbackRuntime();
  private readonly constraints = new AvatarConstraintPipeline();
  private readonly footContacts = new FootContactAnalyzer();
  private readonly stageLocomotion = new StageLocomotionController();
  private lastLocomotionPhase: StageLocomotionPhase | undefined;
  private readonly faceGaze = new FaceGazeRuntime();
  private readonly expressionMixer = new FaceExpressionMixer();
  private readonly secondaryMotion = new SecondaryMotionRuntime();
  private readonly motionPreloader: MotionPreloader;

  constructor(private readonly container: HTMLElement) {
    this.renderer = new WebGLRenderer({ alpha: true, antialias: true, premultipliedAlpha: true });
    this.canvas = this.renderer.domElement;
    this.canvas.className = "pet-avatar-canvas";
    this.canvas.dataset["motionRuntime"] = "v5";
    this.canvas.setAttribute("aria-hidden", "true");
    this.renderer.setClearColor(new Color(0x000000), 0);
    this.renderer.outputColorSpace = SRGBColorSpace;
    this.renderer.toneMapping = NeutralToneMapping;
    this.renderer.toneMappingExposure = 0.92;
    this.loader.register((parser) => new VRMLoaderPlugin(parser));
    this.motionLoader.register((parser) => new VRMAnimationLoaderPlugin(parser));
    this.motionLibrary = new MotionAssetLibrary(
      this.motionLoader,
      (id) => commands.getMotionRuntimeAsset(id),
      {
        read: (cacheKey) => commands.readMotionFeatureIndex({ cacheKey }),
        write: (cacheKey, payload) => commands.writeMotionFeatureIndex({ cacheKey, payload }),
      },
    );
    this.animationGraph = new AnimationGraph(
      { entries: [], transitionProfiles: [] },
      (id, timeMs, mirror) =>
        (this.instance && this.motionLibrary.sample(this.instance.vrm, id, timeMs, mirror)) ?? {
          rotations: new Map(),
          expressions: new Map(),
        },
    );
    this.motionPreloader = new MotionPreloader(this.motionLibrary);
    this.scene.add(new AmbientLight(0xffffff, 0.7));
    this.scene.add(new HemisphereLight(0xf2f0ff, 0x554b68, 0.9));
    const key = new DirectionalLight(0xfff7f1, 1.65);
    key.position.set(3, 5, 5);
    this.scene.add(key);
    const fill = new DirectionalLight(0xb9ccff, 0.45);
    fill.position.set(-4, 2, 3);
    this.scene.add(fill);
    this.container.append(this.canvas);
    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(this.container);
    document.addEventListener("visibilitychange", this.handleVisibilityChange);
    this.resize();
    this.clock.start();
    this.animationFrame = window.requestAnimationFrame(this.animate);
  }

  async load(asset: AvatarRuntimeAsset): Promise<void> {
    const revision = ++this.revision;
    this.stopSpeech(true);
    this.behaviorScheduler.clear();
    this.interactionFeedback.reset();
    this.interactionCorrection = undefined;
    const gltf = await loadAvatarWithDomTextures(this.loader, asset.assetUrl);
    if (revision !== this.revision || this.disposed) {
      deepDisposeAvatarRoot(gltf.scene);
      return;
    }
    const vrm = gltf.userData["vrm"] as VRM | undefined;
    if (!vrm || asset.format === "glb") {
      deepDisposeAvatarRoot(gltf.scene);
      throw new Error("当前运行时仅接受通过 Runtime Ready 检测的 VRM 模型");
    }
    if (vrm && asset.format === "vrm0") VRMUtils.rotateVRM0(vrm);
    const contentRoot = new Group();
    contentRoot.add(gltf.scene);
    const initialBounds = new Box3().setFromObject(contentRoot);
    if (initialBounds.isEmpty()) {
      deepDisposeAvatarRoot(contentRoot);
      throw new Error("模型中没有可显示的网格");
    }
    const center = initialBounds.getCenter(new Vector3());
    contentRoot.position.set(-center.x, -initialBounds.min.y, -center.z);
    const presentationRoot = new Group();
    presentationRoot.add(contentRoot);
    const rootBaseline = capturePresentationRootBaseline(presentationRoot);
    const bounds = new Box3().setFromObject(presentationRoot);
    const height = Math.max(bounds.getSize(new Vector3()).y, 0.001);
    const rawBones = await resolveBones(gltf.parser, asset.profile);
    const bones = resolveControlBones(vrm, rawBones);
    const semanticRegions = buildSemanticRegions(rawBones);
    const restRotations = new Map<string, Quaternion>();
    const restPositions = new Map<string, Vector3>();
    for (const [name, bone] of bones) {
      restRotations.set(name, bone.quaternion.clone());
      restPositions.set(name, bone.position.clone());
    }
    const morphTargets = await resolveMorphTargets(gltf.parser, asset.profile);
    const next: AvatarInstance = {
      asset,
      presentationRoot,
      rootBaseline,
      contentRoot,
      vrm,
      bounds,
      height,
      bones,
      rawBones,
      semanticRegions,
      restRotations,
      restPositions,
      morphTargets,
      soleOffsets: profileSoleOffsets(asset.profile),
    };
    const previous = this.instance;
    presentationRoot.visible = false;
    this.scene.add(presentationRoot);
    this.idleMotion = undefined;
    this.entranceMotion = undefined;
    this.ambientMotion = undefined;
    this.interactionMotion = undefined;
    this.actionRecoveryMotion = undefined;
    this.speechMotion = undefined;
    this.locomotionMotion = undefined;
    this.requestedMotions.clear();
    this.motionOrchestrator.clear();
    this.animationGraph.clear();
    this.headPatActive = false;
    this.motionSetRevision += 1;
    this.motionTransitionSignature = "";
    this.previousResolvedPose = undefined;
    this.resolvedPoseBeforePrevious = undefined;
    this.transitionInertializer.reset();
    this.constraints.reset();
    this.footContacts.reset();
    this.stageLocomotion.reset();
    this.lastLocomotionPhase = undefined;
    this.canvas.dataset["motionLocomotionHistory"] = "";
    this.faceGaze.reset(performance.now());
    this.secondaryMotion.reset();
    this.instance = next;
    const now = performance.now();
    this.startupSequenceComplete = this.entrancePlayedForAvatarIds.has(asset.entryId);
    if (!this.startupSequenceComplete) this.entrancePlayedForAvatarIds.add(asset.entryId);
    this.foregroundWasActive = false;
    this.nextAmbientMotionAt = now + ambientMotionDelayMs(Math.random());
    const baseIdle = this.ensureWaitingIdle(now, false);
    const baseIdleReady = baseIdle ? await this.prepareClip(baseIdle, undefined, true) : false;
    if (revision !== this.revision || this.disposed) return;
    if (!baseIdle || !baseIdleReady) {
      this.scene.remove(presentationRoot);
      this.motionLibrary.clear(vrm);
      disposeAvatarInstance(next);
      this.instance = previous;
      throw new Error("等待动作加载失败；已阻止模型以 T Pose 显示");
    }
    const entrance = this.ensureStartupSequence(now, false);
    if (entrance) await this.prepareClip(entrance);
    if (revision !== this.revision || this.disposed) return;
    this.preloadCatalogDefaults(vrm);
    this.stopSpeech(true);
    if (previous) {
      this.scene.remove(previous.presentationRoot);
      this.motionLibrary.clear(previous.vrm);
      disposeAvatarInstance(previous);
      this.renderer.renderLists.dispose();
    }
    this.frame(presentationRoot);
    this.restoreBasePose(next);
    this.transitionInertializer.reset();
    this.motionTransitionSignature = `${this.motionSetRevision}:${baseIdle.id}`;
    this.update(1 / 60, performance.now());
    presentationRoot.visible = true;
    this.render();
  }

  clear(): void {
    this.revision += 1;
    this.stopSpeech(true);
    this.idleMotion = undefined;
    this.entranceMotion = undefined;
    this.startupSequenceComplete = false;
    this.ambientMotion = undefined;
    this.interactionMotion = undefined;
    this.actionRecoveryMotion = undefined;
    this.speechMotion = undefined;
    this.locomotionMotion = undefined;
    this.requestedMotions.clear();
    this.motionOrchestrator.clear();
    this.animationGraph.clear();
    this.headPatActive = false;
    this.motionTransitionSignature = "";
    this.previousResolvedPose = undefined;
    this.resolvedPoseBeforePrevious = undefined;
    this.constraints.reset();
    this.footContacts.reset();
    this.stageLocomotion.reset();
    this.faceGaze.reset();
    this.secondaryMotion.reset();
    this.behaviorScheduler.clear();
    this.interactionFeedback.reset();
    this.interactionCorrection = undefined;
    this.nextAmbientMotionAt = 0;
    this.recentAmbientMotionIds = [];
    this.foregroundWasActive = false;
    if (this.instance) {
      this.scene.remove(this.instance.presentationRoot);
      this.motionLibrary.clear(this.instance.vrm);
      disposeAvatarInstance(this.instance);
      this.instance = undefined;
      this.renderer.renderLists.dispose();
    }
    this.render();
  }

  setPaused(paused: boolean): void {
    this.visibilityPaused = paused;
    this.syncPausedState();
  }

  setDragging(dragging: boolean): void {
    this.dragging = dragging;
    this.canvas.dataset["motionDrag"] = dragging ? "active" : "released";
    if (dragging) {
      const now = performance.now();
      for (const node of [
        this.entranceMotion,
        this.ambientMotion,
        this.interactionMotion,
        this.actionRecoveryMotion,
        this.locomotionMotion,
        ...this.requestedMotions.values(),
      ])
        this.deactivateMotionNode(node, now);
      this.entranceMotion = undefined;
      this.startupSequenceComplete = true;
      this.ambientMotion = undefined;
      this.interactionMotion = undefined;
      this.actionRecoveryMotion = undefined;
      this.locomotionMotion = undefined;
      this.requestedMotions.clear();
      this.headPatActive = false;
      this.interactionFeedback.setDrag(true, this.dragVelocity, now);
      this.motionSetRevision += 1;
    } else {
      this.dragVelocity.set(0, 0);
      this.interactionFeedback.setDrag(false, this.dragVelocity, performance.now());
    }
  }

  updateWindowMotion(velocityX: number, velocityY: number): void {
    this.dragVelocity.set(
      MathUtils.clamp(velocityX / 1_600, -1, 1),
      MathUtils.clamp(velocityY / 1_600, -1, 1),
    );
    if (this.dragging) this.interactionFeedback.setDrag(true, this.dragVelocity, performance.now());
  }

  setMotionCatalog(snapshot: MotionCatalogSnapshot): void {
    this.motionCatalog = snapshot;
    this.disabledMotionIds = new Set(snapshot.disabledMotionIds);
    this.motionEntries.clear();
    this.transitionProfiles.clear();
    for (const entry of snapshot.entries) this.motionEntries.set(entry.id, entry);
    for (const profile of snapshot.transitionProfiles)
      this.transitionProfiles.set(profile.id, profile);
    this.motionLibrary.setCatalog(snapshot.entries);
    this.animationGraph.setCatalog(snapshot);
    let activeMotionChanged = false;
    const now = performance.now();
    const disable = (node: RuntimeMotionNode | undefined) => {
      if (!node || this.isMotionEnabled(node.id)) return false;
      this.deactivateMotionNode(node, now);
      return true;
    };
    if (disable(this.idleMotion)) {
      this.idleMotion = undefined;
      activeMotionChanged = true;
    }
    if (disable(this.entranceMotion)) {
      this.entranceMotion = undefined;
      this.startupSequenceComplete = true;
      activeMotionChanged = true;
    }
    if (disable(this.ambientMotion)) {
      this.ambientMotion = undefined;
      activeMotionChanged = true;
    }
    if (disable(this.interactionMotion)) {
      this.interactionMotion = undefined;
      this.headPatActive = false;
      activeMotionChanged = true;
    }
    if (disable(this.actionRecoveryMotion)) {
      this.actionRecoveryMotion = undefined;
      activeMotionChanged = true;
    }
    if (disable(this.speechMotion)) {
      this.speechMotion = undefined;
      activeMotionChanged = true;
    }
    if (disable(this.locomotionMotion)) {
      this.locomotionMotion = undefined;
      activeMotionChanged = true;
    }
    for (const [requestId, clip] of this.requestedMotions) {
      if (this.isMotionEnabled(clip.id)) continue;
      this.deactivateMotionNode(clip, now);
      this.requestedMotions.delete(requestId);
      activeMotionChanged = true;
    }
    if (activeMotionChanged) this.motionSetRevision += 1;
    this.ensureStartupSequence(performance.now());
    if (this.instance) this.preloadCatalogDefaults(this.instance.vrm);
  }

  playMotionIntent(request: MotionIntentRequest): boolean {
    const now = performance.now();
    if (!request.active) {
      const accepted = this.motionOrchestrator.submit(request, now);
      const graphRemoved = this.animationGraph.submit(request, now);
      const removed = this.requestedMotions.delete(request.requestId);
      if (removed) this.motionSetRevision += 1;
      return accepted || graphRemoved || removed;
    }
    const entry = this.motionEntries.get(request.motionId);
    if (
      !entry ||
      entry.slot !== request.slot ||
      !this.isMotionEnabled(entry.id) ||
      !request.requestId.trim()
    )
      return false;
    if (!this.motionOrchestrator.submit(request, now)) return false;
    const clip: RuntimeMotionNode = {
      id: entry.id,
      ready: false,
      intent: { ...request, mirror: request.mirror && entry.mirrorable },
    };
    this.requestedMotions.set(request.requestId, clip);
    void this.prepareClip(clip);
    this.motionSetRevision += 1;
    return true;
  }

  private createMotionNode(
    entry: MotionCatalogEntry,
    role: string,
    priority: number,
    mirror: boolean,
    interruptPolicy:
      | MotionIntentRequest["interruptPolicy"]
      | undefined = entry.transitionProfileId === "recovery.fast" ? "immediate" : undefined,
    channelWeights: readonly { channel: BehaviorChannel; weight: number }[] = [],
  ): RuntimeMotionNode {
    return {
      id: entry.id,
      ready: false,
      intent: {
        requestId: `pet:${role}:${++this.motionRequestSequence}`,
        motionId: entry.id,
        slot: entry.slot,
        active: true,
        priority,
        interruptPolicy:
          interruptPolicy ??
          this.transitionProfiles.get(entry.transitionProfileId)?.interruptPolicy ??
          "safe_point",
        mirror: mirror && entry.mirrorable,
        channelWeights: [...channelWeights],
      },
    };
  }

  private deactivateMotionNode(node: RuntimeMotionNode | undefined, now = performance.now()): void {
    if (!node) return;
    const inactive = { ...node.intent, active: false };
    this.motionOrchestrator.submit(inactive, now);
    this.animationGraph.submit(inactive, now);
  }

  applyRuntimeController(request: RuntimeControllerRequest): void {
    const previousSequence = this.controllerSequences.get(request.kind) ?? -1;
    if (request.sequence <= previousSequence) return;
    this.controllerSequences.set(request.kind, request.sequence);
    if (request.kind === "locomotion") {
      if (request.active) this.stageLocomotion.walkTo(request.target[0] ?? 0);
      else this.stageLocomotion.stop();
    } else if (request.kind === "drag") {
      this.setDragging(request.active);
    }
  }

  previewInteraction(region: InteractionRegion): void {
    const direction: -1 | 1 = region.startsWith("left_") ? -1 : 1;
    const now = performance.now();
    this.interactionCorrection = {
      startedAt: now,
      direction,
      strength: region === "foot" || region.endsWith("_leg") ? 1 : 0.68,
    };
    this.interactionFeedback.begin(region, direction, 0.68, now);
    this.canvas.dataset["motionInteraction"] = region;
    this.canvas.dataset["motionInteractionAt"] = String(Math.round(now));
    this.playBoundMotion(region, direction, now, true);
  }

  interruptBehaviors(): void {
    this.entranceMotion = undefined;
    this.startupSequenceComplete = true;
    this.ambientMotion = undefined;
    this.interactionMotion = undefined;
    this.actionRecoveryMotion = undefined;
    this.speechMotion = undefined;
    this.locomotionMotion = undefined;
    this.requestedMotions.clear();
    this.motionOrchestrator.clear();
    this.animationGraph.clear();
    this.behaviorScheduler.clear();
    this.interactionFeedback.reset();
    this.stageLocomotion.stop();
    this.headPatActive = false;
    this.ensureWaitingIdle(performance.now());
    this.motionSetRevision += 1;
  }

  trackCursorAt(clientX: number, clientY: number): void {
    if (!this.instance || this.dragging) return;
    const rect = this.canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;
    const rawYaw = MathUtils.clamp(((clientX - rect.left) / rect.width) * 2 - 1, -1, 1);
    const rawPitch = MathUtils.clamp(-(((clientY - rect.top) / rect.height) * 2 - 1), -1, 1);
    this.faceGaze.attendCursor(rawYaw, rawPitch, performance.now());
  }

  clearCursorAttention(): void {
    this.faceGaze.releaseAttention(performance.now());
  }

  setListening(listening: boolean): void {
    this.listening = listening;
  }

  interactAt(clientX: number, clientY: number): boolean {
    const instance = this.instance;
    if (!instance || performance.now() - this.lastInteractionAt < 500) return false;
    const pointerHit = this.pointerHitAt(instance, clientX, clientY);
    if (!pointerHit) return false;
    const now = performance.now();
    this.lastInteractionAt = now;
    this.faceGaze.attend(this.pointer.x * 24, this.pointer.y * 10, now, 1_100);
    this.interactionFeedback.begin(pointerHit.region, pointerHit.direction, 0.68, now);
    this.canvas.dataset["motionInteraction"] = pointerHit.region;
    this.canvas.dataset["motionInteractionAt"] = String(Math.round(now));
    this.behaviorScheduler.schedule({
      id: `interaction:${now}`,
      category: "interaction",
      slot: "action",
      priority: PET_MOTION_PRIORITIES.interaction,
      interruptPolicy: "safe_point",
      requestedAt: now,
      maximumWaitMs: 120,
      payload: { region: pointerHit.region, direction: pointerHit.direction },
    });
    return true;
  }

  beginHeadPatAt(clientX: number, clientY: number): boolean {
    const instance = this.instance;
    if (!instance) return false;
    const pointerHit = this.pointerHitAt(instance, clientX, clientY);
    if (!pointerHit?.headTopContact) return false;
    const now = performance.now();
    this.headPatActive = true;
    this.headPatDirection = pointerHit.direction;
    this.headPatPressure = 0.25;
    this.interactionFeedback.begin("head_top", pointerHit.direction, 0.25, now);
    this.canvas.dataset["motionInteraction"] = "head_top";
    this.canvas.dataset["motionInteractionAt"] = String(Math.round(now));
    this.playBoundMotion("head_top", pointerHit.direction, now, true);
    this.lastInteractionAt = now;
    this.faceGaze.attend(this.pointer.x * 18, this.pointer.y * 8, now, 1_100);
    return true;
  }

  updateHeadPatAt(
    clientX: number,
    clientY: number,
    durationMs: number,
    speedPixelsPerMs: number,
  ): boolean {
    const instance = this.instance;
    if (!instance) return false;
    const pointerHit = this.pointerHitAt(instance, clientX, clientY);
    if (!pointerHit?.headTopContact) return false;
    const speed = MathUtils.clamp(speedPixelsPerMs / 1.2, 0, 1);
    const pressure = MathUtils.clamp(0.25 + durationMs / 3_600 + speed * 0.2, 0.1, 1);
    this.headPatPressure = MathUtils.clamp(pressure + speed * 0.08, 0.1, 1);
    this.interactionFeedback.update(this.headPatPressure, pointerHit.direction);
    this.faceGaze.attend(this.pointer.x * 18, this.pointer.y * 8, performance.now(), 500);
    return true;
  }

  endHeadPat(): void {
    this.headPatActive = false;
    this.headPatPressure = 0;
    this.interactionFeedback.end(performance.now());
    this.canvas.dataset["motionInteraction"] = "released";
    this.lastInteractionAt = performance.now();
  }

  interact(): void {
    if (!this.instance || performance.now() - this.lastInteractionAt < 500) return;
    const now = performance.now();
    this.lastInteractionAt = now;
    this.faceGaze.attend(0, 0, now, 900);
    const direction = Math.random() < 0.5 ? -1 : 1;
    this.interactionFeedback.begin("generic", direction, 0.68, now);
    this.canvas.dataset["motionInteraction"] = "generic";
    this.canvas.dataset["motionInteractionAt"] = String(Math.round(now));
    this.behaviorScheduler.schedule({
      id: `interaction:${now}`,
      category: "interaction",
      slot: "action",
      priority: PET_MOTION_PRIORITIES.interaction,
      interruptPolicy: "safe_point",
      requestedAt: now,
      maximumWaitMs: 120,
      payload: { region: "generic", direction },
    });
  }

  private pointerHitAt(
    instance: AvatarInstance,
    clientX: number,
    clientY: number,
  ): AvatarPointerHit | undefined {
    const rect = this.canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return undefined;
    this.pointer.set(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      -((clientY - rect.top) / rect.height) * 2 + 1,
    );
    this.raycaster.setFromCamera(this.pointer, this.camera);
    const hit = this.raycaster.intersectObject(instance.contentRoot, true)[0];
    if (!hit) return undefined;
    const center = instance.bounds.getCenter(new Vector3());
    const headTopContact = isNearHeadTopContact(hit.point, instance);
    return {
      region: headTopContact ? "head_top" : classifyHit(hit, instance),
      direction: hit.point.x < center.x ? -1 : 1,
      headTopContact,
    };
  }

  handleSpeechPlayback(event: SpeechPlaybackEvent): void {
    if (event.source !== "pet_turn") return;
    if (isPlaybackOlder(event.playbackId, this.latestPlaybackId)) return;
    if (this.speech && event.sequence <= this.speech.sequence) return;
    const now = performance.now();
    this.canvas.dataset["motionSpeech"] = event.phase;
    if (event.phase === "prepared" && event.timeline) {
      this.latestPlaybackId = event.playbackId;
      this.speech = {
        playbackId: event.playbackId,
        segmentIndex: event.segmentIndex,
        sequence: event.sequence,
        durationMs: event.durationMs,
        mediaPositionMs: 0,
        receivedAt: now,
        timeline: event.timeline,
        playing: false,
        releaseFrom: 0,
      };
      return;
    }
    if (!this.speech || this.speech.playbackId !== event.playbackId) return;
    this.speech.sequence = event.sequence;
    this.speech.mediaPositionMs = Math.min(event.mediaPositionMs, this.speech.durationMs);
    this.speech.receivedAt = now;
    if (event.phase === "playing") {
      this.speech.playing = true;
      this.startSpeechMotion(event.playbackId, now);
    } else if (event.phase === "progress") {
      this.speech.playing = true;
    } else if (
      event.phase === "completed" ||
      event.phase === "stopped" ||
      event.phase === "failed"
    ) {
      this.beginSpeechRelease(now);
    }
  }

  stopSpeech(immediate = false): void {
    if (!immediate && this.speech) {
      this.beginSpeechRelease(performance.now());
      return;
    }
    this.speech = undefined;
    this.canvas.dataset["motionSpeech"] = "idle";
    this.deactivateMotionNode(this.speechMotion);
    this.speechMotion = undefined;
    this.motionSetRevision += 1;
    this.setExpression("aa", 0);
    this.setExpression("ih", 0);
    this.setExpression("ou", 0);
    this.setExpression("ee", 0);
    this.setExpression("oh", 0);
  }

  private beginSpeechRelease(now: number): void {
    if (!this.speech || this.speech.stoppingAt !== undefined) return;
    this.speech.releaseFrom = this.sampleSpeechEnergy(this.speech, now);
    this.speech.stoppingAt = now;
    this.speech.playing = false;
    this.deactivateMotionNode(this.speechMotion, now);
    this.speechMotion = undefined;
    this.motionSetRevision += 1;
  }

  dispose(): void {
    this.disposed = true;
    this.revision += 1;
    window.cancelAnimationFrame(this.animationFrame);
    document.removeEventListener("visibilitychange", this.handleVisibilityChange);
    this.resizeObserver.disconnect();
    this.clear();
    this.renderer.dispose();
    this.renderer.forceContextLoss();
    this.canvas.remove();
  }

  private readonly handleVisibilityChange = () => {
    this.setPaused(document.hidden);
  };

  private syncPausedState(): void {
    const paused = this.visibilityPaused;
    if (paused === this.paused) return;
    this.paused = paused;
    if (paused) {
      this.stopSpeech();
      this.motionTransitionSignature = "";
      this.previousResolvedPose = undefined;
      this.resolvedPoseBeforePrevious = undefined;
      this.constraints.reset();
      this.secondaryMotion.reset();
      this.behaviorScheduler.clear();
    }
    this.clock.stop();
    if (!paused) this.clock.start();
  }

  private readonly animate = () => {
    if (this.disposed) return;
    this.animationFrame = window.requestAnimationFrame(this.animate);
    const delta = Math.min(this.clock.getDelta(), 0.05);
    if (this.paused || document.hidden) return;
    this.update(delta, performance.now());
    this.render();
  };

  private update(delta: number, now: number): void {
    const instance = this.instance;
    if (!instance) return;
    this.canvas.dataset["motionFrameAt"] = String(Math.round(now));
    this.canvas.dataset["motionSlots"] = this.motionOrchestrator
      .winners()
      .map((intent) => intent.slot)
      .join(",");
    let composition: CatalogMotionComposition | undefined;
    let motionFrame: CatalogMotionFrame = {};
    runAvatarFramePipeline({
      sampleAndCompose: () => {
        this.restorePose(instance);
        this.expressionMixer.beginFrame();
        if (this.listening) this.setExpression("relaxed", 0.08, "base");
        restorePresentationRoot(instance.presentationRoot, instance.rootBaseline);
        const stageFrame = this.stageLocomotion.update(delta);
        if (!this.dragging) {
          instance.presentationRoot.position.x =
            instance.rootBaseline.position.x + stageFrame.positionX * instance.height;
          this.updateLocomotionMotion(stageFrame, now);
        }
        this.recordLocomotionPhase(stageFrame.phase);
        this.updateMotionSchedule(now);
        composition = this.composeCatalogMotions(instance, now);
      },
      inertialize: () => {
        if (composition) motionFrame = this.applyInertializedMotion(instance, composition, delta);
      },
      applyInteractionFeedback: () => this.applyContinuousCorrections(instance, now),
      solveFootContactsAndIk: () => {
        instance.presentationRoot.updateWorldMatrix(true, true);
        const contacts = this.footContacts.update(
          instance.bones,
          instance.height,
          0,
          delta,
          instance.soleOffsets,
        );
        this.lastFootContact = contactState(contacts.left.phase, contacts.right.phase);
        const diagnostics = this.constraints.solve(
          {
            bones: instance.bones,
            restRotations: instance.restRotations,
            profile: instance.asset.profile,
            height: instance.height,
            groundY: 0,
          },
          {
            leftFootPhase: contacts.left.phase,
            rightFootPhase: contacts.right.phase,
            footStrength:
              contacts.left.phase === "air" && contacts.right.phase === "air" ? 0 : 0.82,
            endEffectors: [],
            centerOfMass: contacts.centerOfMass,
          },
        );
        this.canvas.dataset["motionFootDriftRatio"] = diagnostics.maxFootDriftNormalized.toFixed(5);
        this.canvas.dataset["motionGroundPenetrationRatio"] =
          diagnostics.groundPenetrationNormalized.toFixed(5);
      },
      applyFaceGazeAndLipSync: () => {
        this.faceGaze.setContext({
          speaking: Boolean(this.speech?.playing),
          sleepiness: 0.08,
          curiosity: this.listening ? 0.82 : 0.58,
          energy: this.speech?.playing ? 0.82 : 0.68,
        });
        const face = this.faceGaze.update(now, delta);
        this.canvas.dataset["motionHeadYaw"] = face.headYaw.toFixed(3);
        this.canvas.dataset["motionHeadPitch"] = face.headPitch.toFixed(3);
        this.applyLookAt(
          instance,
          motionFrame.lookAt?.yawDegrees ?? face.eyeYaw,
          motionFrame.lookAt?.pitchDegrees ?? face.eyePitch,
        );
        this.applyBoneEuler(
          instance,
          "head",
          MathUtils.degToRad(face.headPitch),
          MathUtils.degToRad(face.headYaw),
          0,
        );
        this.applyBoneEuler(instance, "chest", 0, MathUtils.degToRad(face.chestYaw), 0);
        if (face.blink > 0) {
          this.setExpression("blink", face.blink, "blink_viseme");
          this.setExpression("blink_left", face.blink, "blink_viseme");
          this.setExpression("blink_right", face.blink, "blink_viseme");
        }
        this.updateSpeech(instance, now);
        this.flushExpressions(instance);
      },
      updateSpringBones: () =>
        this.secondaryMotion.update(delta, (step) => instance.vrm?.update(step)),
    });
    stabilizePresentationRoot(instance.presentationRoot, instance.rootBaseline, instance.height);
  }

  private restorePose(instance: AvatarInstance): void {
    for (const [name, rotation] of instance.restRotations) {
      instance.bones.get(name)?.quaternion.copy(rotation);
    }
    for (const [name, position] of instance.restPositions) {
      instance.bones.get(name)?.position.copy(position);
    }
    instance.vrm?.expressionManager?.resetValues();
    instance.vrm?.lookAt?.reset();
    for (const target of instance.morphTargets) {
      for (const host of target.hosts) host.influences[host.index] = 0;
    }
  }

  private restoreBasePose(instance: AvatarInstance): void {
    restorePresentationRoot(instance.presentationRoot, instance.rootBaseline);
    this.restorePose(instance);
    instance.vrm?.update(0);
  }

  private composeCatalogMotions(instance: AvatarInstance, now: number): CatalogMotionComposition {
    const graphLayers = this.animationGraph.update(now);
    const composed = composeMotionLayers(instance.restRotations, [
      ...graphLayers.map((layer) => ({
        id: layer.id,
        pose: layer.pose,
        priority: layer.priority,
        weight: layer.weight,
        channels: this.speech?.playing ? { ...layer.channels, mouth: 0 } : layer.channels,
      })),
    ]);
    const expressions = new Map(
      [...composed.expressions].filter(
        ([expression]) => !this.speech?.playing || !isMouthExpression(expression),
      ),
    );
    return {
      pose: {
        rotations: composed.rotations,
        expressions,
        ...(composed.hipsPosition ? { hipsPosition: composed.hipsPosition } : {}),
        ...(!this.speech?.playing && composed.lookAt ? { lookAt: composed.lookAt } : {}),
      },
      signature: `${this.motionSetRevision}:${[
        ...graphLayers.map((layer) => `${layer.id}:${layer.motionId}`),
      ].join("|")}`,
      inertialHalfLives: [...graphLayers].sort((left, right) => right.priority - left.priority)[0]
        ?.inertialHalfLives,
    };
  }

  private applyInertializedMotion(
    instance: AvatarInstance,
    composition: CatalogMotionComposition,
    deltaSeconds: number,
  ): CatalogMotionFrame {
    if (composition.signature !== this.motionTransitionSignature) {
      const current = this.previousResolvedPose ?? composition.pose;
      const currentVelocity = velocityBetweenPoses(
        this.resolvedPoseBeforePrevious,
        current,
        deltaSeconds,
      );
      this.transitionInertializer.capture(
        current,
        composition.pose,
        currentVelocity,
        zeroVelocity(),
      );
      this.motionTransitionSignature = composition.signature;
    }
    const resolved = this.transitionInertializer.apply(
      composition.pose,
      deltaSeconds,
      composition.inertialHalfLives,
    );
    const continuity = limitPoseStep(resolved, this.previousResolvedPose, instance.height);
    this.canvas.dataset["motionBoneStepDeg"] = continuity.boneDegrees.toFixed(3);
    this.canvas.dataset["motionRootStepRatio"] = continuity.rootHeightRatio.toFixed(5);
    this.canvas.dataset["motionLookAtStepDeg"] = continuity.lookAtDegrees.toFixed(3);
    for (const [name, rotation] of resolved.rotations) {
      instance.bones.get(name)?.quaternion.copy(rotation);
    }
    for (const [expression, value] of resolved.expressions) {
      this.setExpression(expression, value, "behavior");
    }
    const hips = instance.bones.get("hips");
    const restHips = instance.restPositions.get("hips");
    if (hips && restHips) {
      hips.position.copy(restHips);
      if (resolved.hipsPosition) hips.position.add(resolved.hipsPosition);
      const horizontalLimit = instance.height * 0.095;
      hips.position.x = MathUtils.clamp(
        hips.position.x,
        restHips.x - horizontalLimit,
        restHips.x + horizontalLimit,
      );
      hips.position.y = MathUtils.clamp(
        hips.position.y,
        restHips.y - instance.height * 0.055,
        restHips.y + instance.height * 0.055,
      );
      hips.position.z = MathUtils.clamp(
        hips.position.z,
        restHips.z - horizontalLimit,
        restHips.z + horizontalLimit,
      );
    }
    this.resolvedPoseBeforePrevious = this.previousResolvedPose;
    this.previousResolvedPose = cloneSampledPose(resolved);
    return {
      ...(resolved.lookAt ? { lookAt: resolved.lookAt } : {}),
    };
  }

  private expireFinishedMotions(now: number): void {
    const expired = (node: RuntimeMotionNode | undefined) =>
      Boolean(node?.ready && !this.animationGraph.has(node.intent.requestId));
    if (expired(this.entranceMotion)) {
      this.deactivateMotionNode(this.entranceMotion, now);
      this.entranceMotion = undefined;
      this.startupSequenceComplete = true;
      this.ensureWaitingIdle(now);
      this.nextAmbientMotionAt = now + ambientMotionDelayMs(Math.random());
      this.motionSetRevision += 1;
    }
    if (expired(this.ambientMotion)) {
      this.deactivateMotionNode(this.ambientMotion, now);
      this.ambientMotion = undefined;
      this.startActionRecovery(now);
      this.motionSetRevision += 1;
    }
    if (expired(this.interactionMotion)) {
      this.deactivateMotionNode(this.interactionMotion, now);
      this.interactionMotion = undefined;
      this.startActionRecovery(now);
      this.motionSetRevision += 1;
    }
    if (expired(this.actionRecoveryMotion)) {
      this.deactivateMotionNode(this.actionRecoveryMotion, now);
      this.actionRecoveryMotion = undefined;
      this.motionSetRevision += 1;
    }
    let requestedExpired = false;
    for (const [requestId, clip] of this.requestedMotions) {
      if (!expired(clip)) continue;
      this.requestedMotions.delete(requestId);
      this.deactivateMotionNode(clip, now);
      requestedExpired = true;
    }
    if (requestedExpired) this.motionSetRevision += 1;
    if (requestedExpired) this.startActionRecovery(now);
  }

  private applyContinuousCorrections(instance: AvatarInstance, now: number): void {
    const feedback = this.interactionFeedback.frame(now);
    if (
      Number(this.canvas.dataset["motionFeedbackAt"] ?? 0) <
      Number(this.canvas.dataset["motionInteractionAt"] ?? 0)
    ) {
      this.canvas.dataset["motionFeedbackAt"] = String(Math.round(now));
    }
    if (feedback.region === "head_top" && feedback.pressure > 0) {
      const pressure = MathUtils.clamp(feedback.pressure, 0, 1);
      this.applyBoneEuler(
        instance,
        "head",
        pressure * 0.055,
        0,
        feedback.direction * pressure * 0.075,
      );
      this.applyBoneEuler(instance, "chest", pressure * 0.018, 0, -feedback.direction * 0.025);
      this.setExpression("happy", 0.28 + pressure * 0.32, "reaction");
    }
    if (this.dragging || feedback.dragVelocity.lengthSq() > 0.0001) {
      const velocity = feedback.dragVelocity;
      this.applyBoneEuler(instance, "hips", 0, 0, -velocity.x * 0.13);
      this.applyBoneEuler(instance, "chest", velocity.y * 0.09, 0, -velocity.x * 0.08);
      this.applyBoneEuler(instance, "head", -velocity.y * 0.055, 0, velocity.x * 0.04);
    }
    const correction = this.interactionCorrection;
    if (correction) {
      const elapsed = now - correction.startedAt;
      const envelope = Math.exp(-elapsed / 180) * Math.sin((elapsed / 520) * Math.PI);
      if (elapsed >= 620) {
        this.interactionCorrection = undefined;
      } else {
        const impulse = correction.direction * correction.strength * envelope;
        this.applyBoneEuler(instance, "hips", 0, 0, -impulse * 0.07);
        this.applyBoneEuler(instance, "chest", 0, impulse * 0.05, -impulse * 0.11);
        this.applyBoneEuler(instance, "head", 0, impulse * 0.035, -impulse * 0.055);
      }
    }
  }

  private applyLookAt(instance: AvatarInstance, yawDegrees: number, pitchDegrees: number): void {
    if (!instance.vrm?.lookAt) return;
    instance.vrm.lookAt.yaw = yawDegrees;
    instance.vrm.lookAt.pitch = pitchDegrees;
  }

  private updateSpeech(instance: AvatarInstance, now: number): void {
    const speech = this.speech;
    if (!speech) return;
    const openness = this.sampleSpeechEnergy(speech, now);
    if (speech.stoppingAt !== undefined && now - speech.stoppingAt >= 120) {
      this.stopSpeech(true);
      return;
    }
    this.setExpression("aa", openness, "blink_viseme");
    this.applyBoneEuler(instance, "head", openness * 0.018, 0, 0);
  }

  private sampleSpeechPosition(now: number): number | undefined {
    const speech = this.speech;
    if (!speech) return undefined;
    if (!speech.playing) return speech.mediaPositionMs;
    return Math.min(
      speech.mediaPositionMs + Math.min(Math.max(now - speech.receivedAt, 0), 45),
      speech.durationMs,
    );
  }

  private sampleSpeechEnergy(speech: SpeechState, now: number): number {
    if (speech.stoppingAt !== undefined) {
      return speechReleaseEnvelope(speech.releaseFrom, now - speech.stoppingAt);
    }
    const position = this.sampleSpeechPosition(now) ?? speech.mediaPositionMs;
    const frame = Math.min(
      Math.floor(position / Math.max(speech.timeline.frameDurationMs, 1)),
      speech.timeline.jawOpen.length - 1,
    );
    return MathUtils.clamp((speech.timeline.jawOpen[Math.max(frame, 0)] ?? 0) / 255, 0, 1);
  }

  private updateMotionSchedule(now: number): void {
    this.expireFinishedMotions(now);
    let foregroundActive = this.hasForegroundMotion();
    if (this.foregroundWasActive && !foregroundActive) {
      this.nextAmbientMotionAt = now + ambientMotionDelayMs(Math.random());
    }
    this.foregroundWasActive = foregroundActive;

    for (const interaction of this.behaviorScheduler.takeReady(
      now,
      this.animationGraph.safeSlots(now),
    )) {
      if (this.startInteraction(interaction.payload.region, interaction.payload.direction, now)) {
        foregroundActive = true;
        this.foregroundWasActive = true;
      }
    }

    if (this.behaviorScheduler.size === 0 && !foregroundActive && now >= this.nextAmbientMotionAt) {
      this.startRandomAmbientMotion(now);
      this.foregroundWasActive = this.hasForegroundMotion();
    }
  }

  private hasForegroundMotion(): boolean {
    return Boolean(
      this.dragging ||
      this.entranceMotion ||
      this.ambientMotion ||
      this.interactionMotion ||
      this.actionRecoveryMotion ||
      this.speechMotion ||
      this.speech?.playing ||
      this.locomotionMotion ||
      this.requestedMotions.size > 0,
    );
  }

  private startRandomAmbientMotion(now: number): void {
    const allCandidates = this.motionCatalog.entries.filter(
      (entry) =>
        ["idle", "gesture", "reaction"].includes(entry.family) &&
        entry.loopMode === "once" &&
        this.isMotionEnabled(entry.id) &&
        /openmaiwaifu/i.test(entry.sourceProject) &&
        entry.rootMode !== "stage" &&
        !/(?:shy|cool).?waiting|appearing|talking|phone|sing|liked/i.test(entry.name),
    );
    const energy = this.speech?.playing ? 0.9 : this.listening ? 0.7 : 0.42;
    const entry = selectAmbientIdle(
      allCandidates,
      this.recentAmbientMotionIds,
      energy,
      Math.random(),
    );
    this.nextAmbientMotionAt = now + ambientMotionDelayMs(Math.random());
    if (!entry) return;
    this.recentAmbientMotionIds = rememberAmbientMotion(
      this.recentAmbientMotionIds,
      entry.id,
      allCandidates.length,
    );
    this.canvas.dataset["motionAmbient"] = entry.id;
    this.deactivateMotionNode(this.ambientMotion, now);
    this.ambientMotion = this.createMotionNode(
      entry,
      "autonomous",
      PET_MOTION_PRIORITIES.autonomous,
      false,
    );
    void this.prepareClip(this.ambientMotion);
    this.motionSetRevision += 1;
  }

  private startActionRecovery(now: number): void {
    const entry = this.motionCatalog.entries.find(
      (candidate) =>
        candidate.motionRole === "action_recover_to_idle" && this.isMotionEnabled(candidate.id),
    );
    if (!entry) return;
    this.deactivateMotionNode(this.actionRecoveryMotion, now);
    this.actionRecoveryMotion = this.createMotionNode(
      entry,
      "action:recover",
      PET_MOTION_PRIORITIES.autonomous,
      false,
      "immediate",
    );
    void this.prepareClip(this.actionRecoveryMotion);
    this.motionSetRevision += 1;
  }

  private enabledBindingFor(region: InteractionRegion) {
    const binding = this.motionCatalog.bindings.find((value) => value.region === region);
    return binding && this.isMotionEnabled(binding.motionId) ? binding : undefined;
  }

  private startInteraction(region: InteractionRegion, direction: -1 | 1, now: number): boolean {
    this.lastInteractionAt = now;
    const side: -1 | 1 = direction < 0 ? -1 : 1;
    if (!this.playBoundMotion(region, side, now)) return false;
    this.interactionCorrection = {
      startedAt: now,
      direction: side,
      strength: region === "foot" || region.endsWith("_leg") ? 1 : 0.68,
    };
    this.interactionFeedback.begin(region, side, 0.68, now);
    this.canvas.dataset["motionInteraction"] = region;
    return true;
  }

  private playBoundMotion(
    region: InteractionRegion,
    direction: -1 | 1,
    now: number,
    bypassCooldown = false,
  ): boolean {
    const binding = this.enabledBindingFor(region);
    if (!binding) return false;
    const lastPlayedAt = this.interactionCooldowns.get(region) ?? Number.NEGATIVE_INFINITY;
    if (!bypassCooldown && now - lastPlayedAt < binding.cooldownMs) return false;
    const id = binding.motionId;
    const entry = this.motionEntries.get(id);
    if (!entry || !this.isMotionEnabled(id)) return false;
    this.interactionCooldowns.set(region, now);
    this.deactivateMotionNode(this.interactionMotion, now);
    this.interactionMotion = this.createMotionNode(
      entry,
      "interaction",
      PET_MOTION_PRIORITIES.interaction,
      binding.mirrorBySide && isLeftRegion(region, direction),
    );
    void this.prepareClip(this.interactionMotion);
    this.motionSetRevision += 1;
    return true;
  }

  private startSpeechMotion(playbackId: string, now: number): void {
    const candidates = this.motionCatalog.entries
      .filter(
        (entry) =>
          this.isMotionEnabled(entry.id) &&
          /^(talking|happy hand gesture|thankful|open palm|standard liked|gentleman liked|ladylike liked)$/i.test(
            entry.name.trim(),
          ),
      )
      .sort((left, right) => Number(right.hasFingerMotion) - Number(left.hasFingerMotion));
    if (candidates.length === 0) {
      candidates.push(
        ...this.motionCatalog.entries.filter(
          (entry) =>
            this.isMotionEnabled(entry.id) &&
            entry.family === "speech" &&
            !/phone|sing/i.test(entry.name),
        ),
      );
    }
    if (candidates.some((entry) => entry.hasFingerMotion)) {
      const withFingers = candidates.filter((entry) => entry.hasFingerMotion);
      candidates.splice(0, candidates.length, ...withFingers);
    }
    if (candidates.length === 0) {
      this.speechMotion = undefined;
      return;
    }
    const entry = candidates[this.speechGestureCursor % candidates.length]!;
    this.speechGestureCursor += 1;
    this.deactivateMotionNode(this.speechMotion, now);
    this.speechMotion = this.createMotionNode(
      entry,
      `speech:${playbackId}`,
      PET_MOTION_PRIORITIES.speech,
      false,
      "safe_point",
      speechChannelWeights(),
    );
    void this.prepareClip(this.speechMotion, playbackId);
    this.motionSetRevision += 1;
  }

  private updateLocomotionMotion(
    frame: {
      phase: StageLocomotionPhase;
      facing: -1 | 1;
      speed: number;
      distanceRemaining: number;
    },
    now: number,
  ): void {
    const { phase, facing } = frame;
    if (phase === "idle") {
      if (this.lastLocomotionPhase && this.lastLocomotionPhase !== "idle") {
        const recovery = this.motionCatalog.entries.find(
          (entry) =>
            entry.motionRole === "locomotion_recover_to_idle" && this.isMotionEnabled(entry.id),
        );
        if (recovery) {
          this.deactivateMotionNode(this.locomotionMotion, now);
          this.locomotionMotion = this.createMotionNode(
            recovery,
            "locomotion:recover",
            PET_MOTION_PRIORITIES.locomotion,
            facing < 0,
            "immediate",
            locomotionChannelWeights(),
          );
          this.locomotionMotion.intent.locomotion = {
            direction: [facing, 0, 0],
            desiredSpeed: 0,
            remainingDistance: 0,
          };
          void this.prepareClip(this.locomotionMotion);
          this.motionSetRevision += 1;
          return;
        }
      }
      if (
        this.locomotionMotion &&
        this.locomotionMotion.intent.motionId !==
          this.motionCatalog.entries.find(
            (entry) => entry.motionRole === "locomotion_recover_to_idle",
          )?.id
      ) {
        this.deactivateMotionNode(this.locomotionMotion, now);
        this.locomotionMotion = undefined;
        this.motionSetRevision += 1;
      }
      return;
    }
    const role =
      phase === "start"
        ? "walk_start"
        : phase === "stop"
          ? "walk_stop"
          : phase === "turn"
            ? facing < 0
              ? "turn_left"
              : "turn_right"
            : "walk_loop";
    const entry =
      this.motionCatalog.entries.find(
        (value) => value.motionRole === role && this.isMotionEnabled(value.id),
      ) ??
      selectMotionForIntent(
        this.motionCatalog.entries.filter((value) => this.isMotionEnabled(value.id)),
        {
          family: "locomotion",
          tags: ["walk"],
          preferFingerMotion: true,
          preferredSource: "OpenMaiWaifu",
        },
      );
    if (!entry) return;
    const mirror = facing < 0 && entry.mirrorable;
    if (this.locomotionMotion?.id === entry.id && this.locomotionMotion.intent.mirror === mirror) {
      this.locomotionMotion.intent.locomotion = {
        direction: [facing, 0, 0],
        desiredSpeed: frame.speed,
        remainingDistance: frame.distanceRemaining,
      };
      this.animationGraph.updateIntent(this.locomotionMotion.intent);
      return;
    }
    this.deactivateMotionNode(this.locomotionMotion, now);
    this.locomotionMotion = this.createMotionNode(
      entry,
      `locomotion:${phase}`,
      PET_MOTION_PRIORITIES.locomotion,
      mirror,
      "safe_point",
      locomotionChannelWeights(),
    );
    this.locomotionMotion.intent.locomotion = {
      direction: [facing, 0, 0],
      desiredSpeed: frame.speed,
      remainingDistance: frame.distanceRemaining,
    };
    void this.prepareClip(this.locomotionMotion);
    this.motionSetRevision += 1;
  }

  private recordLocomotionPhase(phase: StageLocomotionPhase): void {
    this.canvas.dataset["motionLocomotion"] = phase;
    if (phase === this.lastLocomotionPhase) return;
    this.lastLocomotionPhase = phase;
    const history = (this.canvas.dataset["motionLocomotionHistory"] ?? "")
      .split(",")
      .filter(Boolean);
    this.canvas.dataset["motionLocomotionHistory"] = [...history, phase].slice(-8).join(",");
  }

  private ensureWaitingIdle(now: number, prepare = true): RuntimeMotionNode | undefined {
    const candidates = this.motionCatalog.entries.filter(
      (entry) =>
        entry.family === "idle" &&
        entry.loopMode === "loop" &&
        this.isMotionEnabled(entry.id) &&
        /openmaiwaifu/i.test(entry.sourceProject),
    );
    const currentAvailable = candidates.find((entry) => entry.id === this.idleMotion?.id);
    if (currentAvailable?.name.trim().toLowerCase() === "waiting") {
      if (prepare && this.idleMotion && !this.idleMotion.ready) {
        void this.prepareClip(this.idleMotion);
      }
      return this.idleMotion;
    }
    const entry = selectWaitingIdle(candidates);
    if (!entry) return undefined;
    if (this.idleMotion?.id === entry.id) return this.idleMotion;
    const replacement = this.createMotionNode(entry, "base", PET_MOTION_PRIORITIES.idle, false);
    this.idleMotion = replacement;
    if (prepare) void this.prepareClip(this.idleMotion);
    this.motionSetRevision += 1;
    return this.idleMotion;
  }

  private ensureStartupSequence(now: number, prepare = true): RuntimeMotionNode | undefined {
    if (this.startupSequenceComplete) {
      this.ensureWaitingIdle(now);
      return undefined;
    }
    if (this.entranceMotion) {
      if (prepare && !this.entranceMotion.ready) void this.prepareClip(this.entranceMotion);
      return this.entranceMotion;
    }
    const entry = this.motionCatalog.entries.find(
      (candidate) =>
        candidate.name.trim().toLowerCase() === "appearing" && this.isMotionEnabled(candidate.id),
    );
    if (!entry) {
      if (this.motionCatalog.entries.length === 0) return undefined;
      this.startupSequenceComplete = true;
      this.ensureWaitingIdle(now);
      return undefined;
    }
    this.canvas.dataset["motionStartup"] = entry.id;
    this.deactivateMotionNode(this.entranceMotion, now);
    this.entranceMotion = this.createMotionNode(
      entry,
      "entrance",
      PET_MOTION_PRIORITIES.autonomous,
      false,
    );
    if (prepare) void this.prepareClip(this.entranceMotion);
    this.ensureWaitingIdle(now);
    this.motionSetRevision += 1;
    return this.entranceMotion;
  }
  private prepareClip(
    clip: RuntimeMotionNode,
    playbackId?: string,
    primeTransition = false,
  ): Promise<boolean> {
    const instance = this.instance;
    if (!instance) return Promise.resolve(false);
    return this.motionLibrary
      .prepare(instance.vrm, clip.id)
      .then(async () => {
        if (this.disposed || this.instance !== instance) return false;
        const stillCurrent =
          this.idleMotion === clip ||
          this.entranceMotion === clip ||
          this.ambientMotion === clip ||
          this.interactionMotion === clip ||
          this.actionRecoveryMotion === clip ||
          this.locomotionMotion === clip ||
          this.requestedMotions.get(clip.intent.requestId) === clip ||
          (this.speechMotion === clip && (!playbackId || this.speech?.playbackId === playbackId));
        if (!stillCurrent || !this.isMotionEnabled(clip.id)) return false;
        const entry = this.motionEntries.get(clip.id);
        const profile = entry ? this.transitionProfiles.get(entry.transitionProfileId) : undefined;
        const index =
          entry && profile
            ? await this.motionLibrary.prepareFeatureIndex(instance.vrm, entry.id, profile)
            : undefined;
        if (profile && index) {
          const submittedAt = performance.now();
          const currentPose = this.previousResolvedPose;
          const source: MotionFeatureFrame | undefined = currentPose
            ? {
                timeMs: 0,
                loopPhase: 0,
                pose: currentPose,
                velocity: velocityBetweenPoses(
                  this.resolvedPoseBeforePrevious,
                  currentPose,
                  1 / 60,
                ),
                footContact: this.lastFootContact,
                safeEntry: true,
                safeExit: this.animationGraph.safeSlots(submittedAt).has(clip.intent.slot),
              }
            : undefined;
          this.animationGraph.setFeatureIndex(index);
          this.motionOrchestrator.submit(clip.intent, submittedAt);
          if (
            !this.animationGraph.submitWithOptions(clip.intent, submittedAt, source, {
              maximumWaitMs: clip.intent.priority >= PET_MOTION_PRIORITIES.speech ? 120 : 240,
              transitionElapsedMs: primeTransition ? profile.preferredDurationMs : 0,
            })
          )
            throw new Error(`Animation graph rejected ${clip.id}`);
        } else throw new Error(`Motion analysis failed for ${clip.id}`);
        clip.ready = true;
        this.motionSetRevision += 1;
        return true;
      })
      .catch((error: unknown) => {
        console.error(`Unable to prepare motion ${clip.id}`, error);
        if (this.disposed || this.instance !== instance) return false;
        this.recoverFromMotionFailure(clip, performance.now());
        return false;
      });
  }

  private recoverFromMotionFailure(clip: RuntimeMotionNode, now: number): void {
    const entry = this.motionEntries.get(clip.id);
    this.canvas.dataset["motionFallback"] =
      entry?.fallbackMotionId ?? this.idleMotion?.id ?? "idle";
    if (this.idleMotion === clip) {
      this.motionSetRevision += 1;
      return;
    }
    if (this.entranceMotion === clip) {
      this.entranceMotion = undefined;
      this.startupSequenceComplete = true;
      this.nextAmbientMotionAt = now + ambientMotionDelayMs(Math.random());
    }
    if (this.ambientMotion === clip) this.ambientMotion = undefined;
    if (this.interactionMotion === clip) {
      this.interactionMotion = undefined;
      this.interactionFeedback.end(now);
    }
    if (this.actionRecoveryMotion === clip) this.actionRecoveryMotion = undefined;
    if (this.speechMotion === clip) this.speechMotion = undefined;
    if (this.locomotionMotion === clip) {
      this.locomotionMotion = undefined;
      this.stageLocomotion.stop();
    }
    if (this.requestedMotions.get(clip.intent.requestId) === clip) {
      this.requestedMotions.delete(clip.intent.requestId);
    }
    this.deactivateMotionNode(clip, now);
    this.ensureWaitingIdle(now);
    this.motionSetRevision += 1;
  }

  private preloadCatalogDefaults(vrm: VRM): void {
    void this.motionPreloader
      .preloadCore(
        vrm,
        this.motionCatalog.entries.filter((entry) => this.isMotionEnabled(entry.id)),
      )
      .catch((error: unknown) => console.error("Unable to preload avatar motions", error));
  }

  private isMotionEnabled(id: string): boolean {
    return (
      !this.disabledMotionIds.has(id) && this.motionEntries.get(id)?.analysisStatus === "ready"
    );
  }

  private applyBoneEuler(
    instance: AvatarInstance,
    name: string,
    x: number,
    y: number,
    z: number,
  ): void {
    const bone = instance.bones.get(name);
    if (!bone) return;
    const correction = 1;
    const armLimit = MathUtils.degToRad(78);
    const adjustedZ = name.includes("upper_arm")
      ? MathUtils.clamp(z * correction, -armLimit, armLimit)
      : z * correction;
    applyCanonicalBoneEuler(
      instance.contentRoot,
      bone,
      this.rotationEuler.set(x * correction, y * correction, adjustedZ, "XYZ"),
      this.rotationScratch,
    );
  }

  private setExpression(
    expression: string,
    weight: number,
    layer: ExpressionLayer = "behavior",
  ): void {
    this.expressionMixer.set(layer, expression, weight);
  }

  private flushExpressions(instance: AvatarInstance): void {
    for (const [expression, weight] of this.expressionMixer.resolve()) {
      const vrmExpression =
        expression === "blink_left"
          ? "blinkLeft"
          : expression === "blink_right"
            ? "blinkRight"
            : expression;
      if (instance.vrm?.expressionManager?.getExpression(vrmExpression)) {
        instance.vrm.expressionManager.setValue(vrmExpression, MathUtils.clamp(weight, 0, 1));
        continue;
      }
      for (const target of instance.morphTargets) {
        if (target.expression !== expression) continue;
        for (const host of target.hosts) host.influences[host.index] = weight;
      }
    }
  }

  private frame(root: Object3D): void {
    const bounds = new Box3().setFromObject(root);
    const size = bounds.getSize(new Vector3());
    const targetY = size.y * 0.48;
    const aspect = Math.max(
      this.container.clientWidth / Math.max(this.container.clientHeight, 1),
      0.1,
    );
    const halfFov = MathUtils.degToRad(this.camera.fov * 0.5);
    const distanceForHeight = size.y / (2 * Math.tan(halfFov));
    const distanceForWidth = size.x / (2 * Math.tan(halfFov) * aspect);
    const distance = Math.max(distanceForHeight, distanceForWidth, size.z) * 1.16 + size.z * 0.5;
    this.camera.position.set(0, targetY, distance);
    this.camera.near = Math.max(distance / 100, 0.001);
    this.camera.far = Math.max(distance * 100, 100);
    this.camera.lookAt(0, targetY, 0);
    this.camera.updateProjectionMatrix();
  }

  private resize(): void {
    const width = Math.max(this.container.clientWidth, 1);
    const height = Math.max(this.container.clientHeight, 1);
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / height;
    if (this.instance) this.frame(this.instance.presentationRoot);
    this.render();
  }

  private render(): void {
    this.renderer.render(this.scene, this.camera);
  }
}

/**
 * Applies a semantic humanoid rotation in the avatar's coordinate frame. This keeps ordinary GLB
 * bones with Blender/Mixamo-specific local axes from being treated as if every joint used VRM's
 * canonical local basis.
 */
async function resolveBones(
  parser: { getDependency: (type: string, index: number) => Promise<unknown> },
  profile: AvatarAdaptationProfile,
): Promise<Map<string, Object3D>> {
  const result = new Map<string, Object3D>();
  await Promise.all(
    (profile.bones ?? []).map(async (binding) => {
      if (binding.nodeIndex === undefined || !binding.bone) return;
      const object = (await parser.getDependency("node", binding.nodeIndex)) as Object3D | null;
      if (object) {
        object.userData["gltfNodeIndex"] = binding.nodeIndex;
        result.set(binding.bone, object);
      }
    }),
  );
  return result;
}

const VRM_BONE_NAMES: Record<string, VRMHumanBoneName> = {
  hips: "hips",
  spine: "spine",
  chest: "chest",
  upper_chest: "upperChest",
  neck: "neck",
  head: "head",
  jaw: "jaw",
  left_shoulder: "leftShoulder",
  left_upper_arm: "leftUpperArm",
  left_lower_arm: "leftLowerArm",
  left_hand: "leftHand",
  right_shoulder: "rightShoulder",
  right_upper_arm: "rightUpperArm",
  right_lower_arm: "rightLowerArm",
  right_hand: "rightHand",
  left_upper_leg: "leftUpperLeg",
  left_lower_leg: "leftLowerLeg",
  left_foot: "leftFoot",
  left_toes: "leftToes",
  right_upper_leg: "rightUpperLeg",
  right_lower_leg: "rightLowerLeg",
  right_foot: "rightFoot",
  right_toes: "rightToes",
  left_eye: "leftEye",
  right_eye: "rightEye",
  left_thumb_proximal: "leftThumbMetacarpal",
  left_thumb_intermediate: "leftThumbProximal",
  left_thumb_distal: "leftThumbDistal",
  left_index_proximal: "leftIndexProximal",
  left_index_intermediate: "leftIndexIntermediate",
  left_index_distal: "leftIndexDistal",
  left_middle_proximal: "leftMiddleProximal",
  left_middle_intermediate: "leftMiddleIntermediate",
  left_middle_distal: "leftMiddleDistal",
  left_ring_proximal: "leftRingProximal",
  left_ring_intermediate: "leftRingIntermediate",
  left_ring_distal: "leftRingDistal",
  left_little_proximal: "leftLittleProximal",
  left_little_intermediate: "leftLittleIntermediate",
  left_little_distal: "leftLittleDistal",
  right_thumb_proximal: "rightThumbMetacarpal",
  right_thumb_intermediate: "rightThumbProximal",
  right_thumb_distal: "rightThumbDistal",
  right_index_proximal: "rightIndexProximal",
  right_index_intermediate: "rightIndexIntermediate",
  right_index_distal: "rightIndexDistal",
  right_middle_proximal: "rightMiddleProximal",
  right_middle_intermediate: "rightMiddleIntermediate",
  right_middle_distal: "rightMiddleDistal",
  right_ring_proximal: "rightRingProximal",
  right_ring_intermediate: "rightRingIntermediate",
  right_ring_distal: "rightRingDistal",
  right_little_proximal: "rightLittleProximal",
  right_little_intermediate: "rightLittleIntermediate",
  right_little_distal: "rightLittleDistal",
};

function resolveControlBones(vrm: VRM | undefined, rawBones: Map<string, Object3D>) {
  if (!vrm) return new Map(rawBones);
  const result = new Map<string, Object3D>();
  for (const [name, rawBone] of rawBones) {
    const humanBoneName = VRM_BONE_NAMES[name];
    const normalized = humanBoneName
      ? vrm.humanoid.getNormalizedBoneNode(humanBoneName)
      : undefined;
    result.set(name, normalized ?? rawBone);
  }
  for (const [name, humanBoneName] of Object.entries(VRM_BONE_NAMES)) {
    if (result.has(name)) continue;
    const normalized = vrm.humanoid.getNormalizedBoneNode(humanBoneName);
    if (normalized) result.set(name, normalized);
  }
  return result;
}

async function resolveMorphTargets(
  parser: { getDependency: (type: string, index: number) => Promise<unknown> },
  profile: AvatarAdaptationProfile,
): Promise<MorphTarget[]> {
  const targets: MorphTarget[] = [];
  for (const binding of profile.expressions ?? []) {
    const node = (await parser.getDependency("node", binding.nodeIndex)) as Object3D | null;
    if (!node) continue;
    const hosts: MorphTarget["hosts"] = [];
    node.traverse((object) => {
      const mesh = object as Mesh & { morphTargetInfluences?: number[] };
      if (mesh.morphTargetInfluences && binding.morphIndex < mesh.morphTargetInfluences.length) {
        hosts.push({ influences: mesh.morphTargetInfluences, index: binding.morphIndex });
      }
    });
    if (hosts.length > 0) targets.push({ expression: binding.expression, hosts });
  }
  return targets;
}

function buildSemanticRegions(bones: Map<string, Object3D>): Map<Object3D, InteractionRegion> {
  const result = new Map<Object3D, InteractionRegion>();
  for (const [name, bone] of bones) {
    const side = name.startsWith("left_") ? "left" : name.startsWith("right_") ? "right" : "";
    const region: InteractionRegion =
      name === "head"
        ? "face"
        : name.includes("hand") && side
          ? `${side}_hand`
          : name.includes("arm") && side
            ? `${side}_arm`
            : name.includes("leg") && side
              ? `${side}_leg`
              : name.includes("foot") || name.includes("toes")
                ? "foot"
                : name === "chest" || name === "upper_chest"
                  ? "chest"
                  : name === "spine"
                    ? "belly"
                    : name === "hips"
                      ? "hips"
                      : "generic";
    if (region !== "generic") result.set(bone, region);
  }
  return result;
}

export function classifyHit(
  hit: Intersection<Object3D>,
  instance: Pick<AvatarInstance, "semanticRegions" | "bounds">,
): InteractionRegion {
  const dominantBone = dominantSkinBone(hit);
  const semantic = semanticRegionForObject(dominantBone ?? hit.object, instance.semanticRegions);
  if (semantic) return semantic;

  const objectName = normalizeObjectName(hit.object.name);
  if (objectName.includes("head") || objectName.includes("face") || objectName.includes("hair")) {
    return "face";
  }

  const size = instance.bounds.getSize(new Vector3());
  const center = instance.bounds.getCenter(new Vector3());
  const normalizedY = (hit.point.y - instance.bounds.min.y) / Math.max(size.y, 0.001);
  const normalizedX = (hit.point.x - center.x) / Math.max(size.x * 0.5, 0.001);
  return classifyRelativeHit(normalizedX, normalizedY);
}

function isNearHeadTopContact(point: Vector3, instance: AvatarInstance): boolean {
  const contact = instance.asset.profile.contacts?.find((value) => value.id === "head_top");
  if (!contact) return false;
  const bone = instance.bones.get(contact.bone);
  if (!bone) return false;
  const worldPoint = bone.localToWorld(
    new Vector3(
      finiteNumber(contact.localPosition[0], 0),
      finiteNumber(contact.localPosition[1], 0),
      finiteNumber(contact.localPosition[2], 0),
    ),
  );
  const worldScale = bone.getWorldScale(new Vector3());
  const radius =
    finiteNumber(contact.radius, 0) *
    Math.max(Math.abs(worldScale.x), Math.abs(worldScale.y), Math.abs(worldScale.z));
  return point.distanceTo(worldPoint) <= Math.max(radius * 1.6, instance.height * 0.025);
}

function dominantSkinBone(hit: Intersection<Object3D>): Object3D | undefined {
  if (!(hit.object instanceof SkinnedMesh) || !hit.face) return undefined;
  const indices = hit.object.geometry.getAttribute("skinIndex");
  const weights = hit.object.geometry.getAttribute("skinWeight");
  if (!indices || !weights) return undefined;
  let bestWeight = 0;
  let bestBone: number | undefined;
  for (const vertex of [hit.face.a, hit.face.b, hit.face.c]) {
    const vertexIndices = [
      indices.getX(vertex),
      indices.getY(vertex),
      indices.getZ(vertex),
      indices.getW(vertex),
    ];
    const vertexWeights = [
      weights.getX(vertex),
      weights.getY(vertex),
      weights.getZ(vertex),
      weights.getW(vertex),
    ];
    for (let component = 0; component < 4; component += 1) {
      const weight = vertexWeights[component] ?? 0;
      if (weight > bestWeight) {
        bestWeight = weight;
        bestBone = Math.round(vertexIndices[component] ?? -1);
      }
    }
  }
  return bestBone === undefined ? undefined : hit.object.skeleton.bones[bestBone];
}

function semanticRegionForObject(
  object: Object3D | undefined,
  regions: Map<Object3D, InteractionRegion>,
): InteractionRegion | undefined {
  let cursor = object;
  while (cursor) {
    const region = regions.get(cursor);
    if (region) return region;
    cursor = cursor.parent ?? undefined;
  }
  return undefined;
}

function normalizeObjectName(name: string): string {
  return name.toLowerCase().replaceAll(/[^a-z0-9]/g, "");
}

function disposeAvatarInstance(instance: AvatarInstance): void {
  instance.vrm?.springBoneManager?.reset();
  deepDisposeAvatarRoot(instance.presentationRoot);
}
