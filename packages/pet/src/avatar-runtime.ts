import type {
  AvatarAdaptationProfile,
  AvatarRuntimeAsset,
  ClipMotionRequest,
  InteractionRegion,
  MotionCatalogEntry,
  MotionCatalogSnapshot,
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
  classifyRelativeHit,
  canStartQueuedInteraction,
  selectAmbientIdle,
  selectCoolIdle,
  isPlaybackOlder,
  speechReleaseEnvelope,
} from "./avatar-runtime-logic";
import { PoseTransitionInertializer, estimatePoseAngularVelocity } from "./animation-graph";
import { FaceExpressionMixer, FaceGazeRuntime, type ExpressionLayer } from "./face-gaze-runtime";
import {
  AvatarConstraintPipeline,
  FootContactAnalyzer,
  MotionAssetLibrary,
  StageLocomotionController,
  applyCanonicalBoneEuler,
  channelsForFullBody,
  channelsForSpeechBody,
  composeMotionLayers,
  deepDisposeAvatarRoot,
  loadAvatarWithDomTextures,
  selectMotionForIntent,
  type MotionChannelWeights,
  type FootSoleOffsets,
  type StageLocomotionPhase,
  type SampledMotionPose,
} from "@hachimi/avatar-motion-runtime";
import { SecondaryMotionRuntime } from "./secondary-motion-runtime";

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

export interface PresentationRootBaseline {
  position: Vector3;
  quaternion: Quaternion;
  scale: Vector3;
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

interface AvatarPointerHit {
  region: InteractionRegion;
  direction: -1 | 1;
  headTopContact: boolean;
}

interface ActiveMotionClip {
  id: string;
  requestId?: string;
  startedAt: number;
  mirror: boolean;
  ready: boolean;
  priority?: number;
  channelWeights?: MotionChannelWeights;
}

interface WeightedMotionSample {
  clip: ActiveMotionClip;
  entry: MotionCatalogEntry;
  pose: SampledMotionPose;
  weight: number;
  priority: number;
  channelWeights: MotionChannelWeights;
}

interface CatalogMotionFrame {
  lookAt?: { yawDegrees: number; pitchDegrees: number };
}

const AMBIENT_MOTION_INTERVAL_MS = 15_000;
const IDLE_RETURN_SETTLE_MS = 350;

export class AvatarRuntime {
  readonly canvas: HTMLCanvasElement;
  private readonly renderer: WebGLRenderer;
  private readonly scene = new Scene();
  private readonly camera = new PerspectiveCamera(28, 1, 0.01, 10_000);
  private readonly loader = new GLTFLoader();
  private readonly motionLoader = new GLTFLoader();
  private readonly motionLibrary: MotionAssetLibrary;
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
  private pendingInteraction:
    | { executeAt: number; region: InteractionRegion; direction: -1 | 1 }
    | undefined;
  private motionCatalog: MotionCatalogSnapshot = {
    entries: [],
    bindings: [],
    disabledMotionIds: [],
  };
  private readonly motionEntries = new Map<string, MotionCatalogEntry>();
  private disabledMotionIds = new Set<string>();
  private readonly interactionCooldowns = new Map<InteractionRegion, number>();
  private readonly entrancePlayedForAvatarIds = new Set<string>();
  private idleMotion: ActiveMotionClip | undefined;
  private entranceMotion: ActiveMotionClip | undefined;
  private startupSequenceComplete = false;
  private ambientMotion: ActiveMotionClip | undefined;
  private nextAmbientMotionAt = 0;
  private lastAmbientMotionId: string | undefined;
  private idleReturnReadyAt = 0;
  private foregroundWasActive = false;
  private interactionMotion: ActiveMotionClip | undefined;
  private speechMotion: ActiveMotionClip | undefined;
  private locomotionMotion: ActiveMotionClip | undefined;
  private readonly requestedMotions = new Map<string, ActiveMotionClip>();
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
  private readonly transitionInertializer = new PoseTransitionInertializer();
  private motionTransitionSignature = "";
  private previousVisualPose: Map<string, Quaternion> | undefined;
  private readonly constraints = new AvatarConstraintPipeline();
  private readonly footContacts = new FootContactAnalyzer();
  private readonly stageLocomotion = new StageLocomotionController();
  private readonly faceGaze = new FaceGazeRuntime();
  private readonly expressionMixer = new FaceExpressionMixer();
  private readonly secondaryMotion = new SecondaryMotionRuntime();

  constructor(private readonly container: HTMLElement) {
    this.renderer = new WebGLRenderer({ alpha: true, antialias: true, premultipliedAlpha: true });
    this.canvas = this.renderer.domElement;
    this.canvas.className = "pet-avatar-canvas";
    this.canvas.setAttribute("aria-hidden", "true");
    this.renderer.setClearColor(new Color(0x000000), 0);
    this.renderer.outputColorSpace = SRGBColorSpace;
    this.renderer.toneMapping = NeutralToneMapping;
    this.renderer.toneMappingExposure = 0.92;
    this.loader.register((parser) => new VRMLoaderPlugin(parser));
    this.motionLoader.register((parser) => new VRMAnimationLoaderPlugin(parser));
    this.motionLibrary = new MotionAssetLibrary(this.motionLoader, (id) =>
      commands.getMotionRuntimeAsset(id),
    );
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
    this.pendingInteraction = undefined;
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
    this.speechMotion = undefined;
    this.locomotionMotion = undefined;
    this.requestedMotions.clear();
    this.headPatActive = false;
    this.motionSetRevision += 1;
    this.motionTransitionSignature = "";
    this.previousVisualPose = undefined;
    this.transitionInertializer.reset();
    this.constraints.reset();
    this.footContacts.reset();
    this.stageLocomotion.reset();
    this.faceGaze.reset(performance.now());
    this.secondaryMotion.reset();
    this.instance = next;
    const now = performance.now();
    this.startupSequenceComplete = this.entrancePlayedForAvatarIds.has(asset.entryId);
    if (!this.startupSequenceComplete) this.entrancePlayedForAvatarIds.add(asset.entryId);
    this.foregroundWasActive = false;
    this.idleReturnReadyAt = now;
    this.nextAmbientMotionAt = now + AMBIENT_MOTION_INTERVAL_MS;
    const baseIdle = this.ensureCoolIdle(now, false);
    const baseIdleReady = baseIdle ? await this.prepareClip(baseIdle) : false;
    if (revision !== this.revision || this.disposed) return;
    if (!baseIdle || !baseIdleReady) {
      this.scene.remove(presentationRoot);
      this.motionLibrary.clear(vrm);
      disposeAvatarInstance(next);
      this.instance = previous;
      throw new Error("酷系待机加载失败；已阻止模型以 T Pose 显示");
    }
    const baseIdleEntry = this.motionEntries.get(baseIdle.id);
    baseIdle.startedAt =
      performance.now() - Math.max(baseIdleEntry?.transitionInMs ?? IDLE_RETURN_SETTLE_MS, 1);
    this.ensureStartupSequence(now);
    this.preloadCatalogDefaults(vrm);
    this.stopSpeech(true);
    presentationRoot.visible = true;
    if (previous) {
      this.scene.remove(previous.presentationRoot);
      this.motionLibrary.clear(previous.vrm);
      disposeAvatarInstance(previous);
      this.renderer.renderLists.dispose();
    }
    this.frame(presentationRoot);
    this.restoreBasePose(next);
    // The first visible frame must be the fully weighted cool idle. Matching
    // the initial signature bypasses inertialization from the normalized rest pose.
    this.transitionInertializer.reset();
    this.motionTransitionSignature = `${this.motionSetRevision}:${baseIdle.id}`;
    this.update(1 / 60, performance.now());
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
    this.speechMotion = undefined;
    this.locomotionMotion = undefined;
    this.requestedMotions.clear();
    this.headPatActive = false;
    this.motionTransitionSignature = "";
    this.previousVisualPose = undefined;
    this.constraints.reset();
    this.footContacts.reset();
    this.stageLocomotion.reset();
    this.faceGaze.reset();
    this.secondaryMotion.reset();
    this.pendingInteraction = undefined;
    this.interactionCorrection = undefined;
    this.nextAmbientMotionAt = 0;
    this.lastAmbientMotionId = undefined;
    this.idleReturnReadyAt = 0;
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
    if (dragging) {
      this.entranceMotion = undefined;
      this.startupSequenceComplete = true;
      this.ambientMotion = undefined;
      this.interactionMotion = undefined;
      this.headPatActive = false;
      this.motionSetRevision += 1;
    } else {
      this.dragVelocity.set(0, 0);
    }
  }

  updateWindowMotion(velocityX: number, velocityY: number): void {
    this.dragVelocity.set(
      MathUtils.clamp(velocityX / 1_600, -1, 1),
      MathUtils.clamp(velocityY / 1_600, -1, 1),
    );
  }

  setMotionCatalog(snapshot: MotionCatalogSnapshot): void {
    this.motionCatalog = snapshot;
    this.disabledMotionIds = new Set(snapshot.disabledMotionIds);
    this.motionEntries.clear();
    for (const entry of snapshot.entries) this.motionEntries.set(entry.id, entry);
    this.motionLibrary.setCatalog(snapshot.entries);
    let activeMotionChanged = false;
    if (this.idleMotion && !this.isMotionEnabled(this.idleMotion.id)) {
      this.idleMotion = undefined;
      activeMotionChanged = true;
    }
    if (this.entranceMotion && !this.isMotionEnabled(this.entranceMotion.id)) {
      this.entranceMotion = undefined;
      this.startupSequenceComplete = true;
      activeMotionChanged = true;
    }
    if (this.ambientMotion && !this.isMotionEnabled(this.ambientMotion.id)) {
      this.ambientMotion = undefined;
      activeMotionChanged = true;
    }
    if (this.interactionMotion && !this.isMotionEnabled(this.interactionMotion.id)) {
      this.interactionMotion = undefined;
      this.headPatActive = false;
      activeMotionChanged = true;
    }
    if (this.speechMotion && !this.isMotionEnabled(this.speechMotion.id)) {
      this.speechMotion = undefined;
      activeMotionChanged = true;
    }
    if (this.locomotionMotion && !this.isMotionEnabled(this.locomotionMotion.id)) {
      this.locomotionMotion = undefined;
      activeMotionChanged = true;
    }
    for (const [requestId, clip] of this.requestedMotions) {
      if (this.isMotionEnabled(clip.id)) continue;
      this.requestedMotions.delete(requestId);
      activeMotionChanged = true;
    }
    if (activeMotionChanged) this.motionSetRevision += 1;
    this.ensureStartupSequence(performance.now());
    if (this.instance) this.preloadCatalogDefaults(this.instance.vrm);
  }

  playClipMotion(request: ClipMotionRequest): boolean {
    if (!request.active) {
      const removed = this.requestedMotions.delete(request.requestId);
      if (removed) this.motionSetRevision += 1;
      return removed;
    }
    const entry = this.motionEntries.get(request.motionId);
    if (!entry || !this.isMotionEnabled(entry.id) || !request.requestId.trim()) return false;
    const channelWeights = Object.fromEntries(
      request.channelWeights.map(({ channel, weight }) => [
        channel,
        MathUtils.clamp(weight / 1_000, 0, 1),
      ]),
    ) as MotionChannelWeights;
    const clip: ActiveMotionClip = {
      id: entry.id,
      requestId: request.requestId,
      startedAt: performance.now(),
      mirror: request.mirror && entry.mirrorable,
      ready: false,
      priority: request.priority,
      channelWeights:
        Object.keys(channelWeights).length > 0 ? channelWeights : channelsForFullBody(),
    };
    this.requestedMotions.set(request.requestId, clip);
    void this.prepareClip(clip);
    this.motionSetRevision += 1;
    return true;
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
    this.playBoundMotion(region, direction, now, true);
  }

  interruptBehaviors(): void {
    this.entranceMotion = undefined;
    this.startupSequenceComplete = true;
    this.ambientMotion = undefined;
    this.interactionMotion = undefined;
    this.speechMotion = undefined;
    this.locomotionMotion = undefined;
    this.requestedMotions.clear();
    this.stageLocomotion.stop();
    this.headPatActive = false;
    this.ensureCoolIdle(performance.now());
    this.motionSetRevision += 1;
  }

  trackCursorAt(clientX: number, clientY: number): void {
    if (!this.instance || this.dragging) return;
    const rect = this.canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;
    const rawYaw = MathUtils.clamp(((clientX - rect.left) / rect.width) * 2 - 1, -1, 1);
    const rawPitch = MathUtils.clamp(-(((clientY - rect.top) / rect.height) * 2 - 1), -1, 1);
    const yaw = Math.abs(rawYaw) < 0.055 ? 0 : rawYaw;
    const pitch = Math.abs(rawPitch) < 0.07 ? 0 : rawPitch;
    this.faceGaze.attend(yaw * 28, pitch * 16, performance.now(), 260);
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
    if (!pointerHit || !this.enabledBindingFor(pointerHit.region)) return false;
    const now = performance.now();
    this.lastInteractionAt = now;
    this.faceGaze.attend(this.pointer.x * 24, this.pointer.y * 10, now, 1_100);
    this.pendingInteraction = {
      executeAt: now + 130,
      region: pointerHit.region,
      direction: pointerHit.direction,
    };
    return true;
  }

  beginHeadPatAt(clientX: number, clientY: number): boolean {
    const instance = this.instance;
    if (!instance) return false;
    const pointerHit = this.pointerHitAt(instance, clientX, clientY);
    if (!pointerHit?.headTopContact || !this.enabledBindingFor("head_top")) return false;
    const now = performance.now();
    if (this.hasForegroundMotion()) {
      this.pendingInteraction = {
        executeAt: now + 130,
        region: "head_top",
        direction: pointerHit.direction,
      };
      this.lastInteractionAt = now;
      return true;
    }
    this.pendingInteraction = undefined;
    this.headPatActive = true;
    this.headPatDirection = pointerHit.direction;
    this.headPatPressure = 0.25;
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
    this.faceGaze.attend(this.pointer.x * 18, this.pointer.y * 8, performance.now(), 500);
    return true;
  }

  endHeadPat(): void {
    this.headPatActive = false;
    this.headPatPressure = 0;
    this.lastInteractionAt = performance.now();
  }

  interact(): void {
    if (
      !this.instance ||
      !this.enabledBindingFor("generic") ||
      performance.now() - this.lastInteractionAt < 500
    )
      return;
    const now = performance.now();
    this.lastInteractionAt = now;
    this.faceGaze.attend(0, 0, now, 900);
    this.pendingInteraction = {
      executeAt: now + 130,
      region: "generic",
      direction: Math.random() < 0.5 ? -1 : 1,
    };
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
      this.previousVisualPose = undefined;
      this.constraints.reset();
      this.secondaryMotion.reset();
      this.pendingInteraction = undefined;
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
    const previousVisualPose = new Map(
      [...instance.bones].map(([name, bone]) => [name, bone.quaternion.clone()] as const),
    );
    const visualAngularVelocity = estimatePoseAngularVelocity(
      this.previousVisualPose,
      previousVisualPose,
      delta,
    );
    this.previousVisualPose = new Map(
      [...previousVisualPose].map(([name, rotation]) => [name, rotation.clone()] as const),
    );
    this.restorePose(instance);
    this.expressionMixer.beginFrame();
    if (this.listening) this.setExpression("relaxed", 0.08, "base");
    restorePresentationRoot(instance.presentationRoot, instance.rootBaseline);
    const stageFrame = this.stageLocomotion.update(delta);
    if (!this.dragging) {
      instance.presentationRoot.position.x =
        instance.rootBaseline.position.x + stageFrame.positionX * instance.height;
      this.updateLocomotionMotion(stageFrame.phase, stageFrame.facing, now);
    }
    this.updateMotionSchedule(now);
    const motionFrame = this.applyCatalogMotions(
      instance,
      now,
      previousVisualPose,
      visualAngularVelocity,
      delta,
    );
    this.applyContinuousCorrections(instance);
    instance.presentationRoot.updateWorldMatrix(true, true);
    const contacts = this.footContacts.update(
      instance.bones,
      instance.height,
      0,
      delta,
      instance.soleOffsets,
    );
    this.constraints.solve(
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
        footStrength: contacts.left.phase === "air" && contacts.right.phase === "air" ? 0 : 0.82,
        endEffectors: [],
        centerOfMass: contacts.centerOfMass,
      },
    );
    this.faceGaze.setContext({
      speaking: Boolean(this.speech?.playing),
      sleepiness: 0.08,
      curiosity: this.listening ? 0.82 : 0.58,
      energy: this.speech?.playing ? 0.82 : 0.68,
    });
    const face = this.faceGaze.update(now, delta);
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
    this.secondaryMotion.update(delta, (step) => instance.vrm?.update(step));
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

  private applyCatalogMotions(
    instance: AvatarInstance,
    now: number,
    previousVisualPose: ReadonlyMap<string, Quaternion>,
    visualAngularVelocity: ReadonlyMap<string, Vector3>,
    deltaSeconds: number,
  ): CatalogMotionFrame {
    const samples = this.collectMotionSamples(instance, now);
    const composed = composeMotionLayers(
      instance.restRotations,
      samples.map((sample) => ({
        id: sample.clip.requestId ?? `${sample.priority}:${sample.entry.id}`,
        pose: sample.pose,
        priority: sample.priority,
        weight: sample.weight,
        channels: this.speech?.playing
          ? { ...sample.channelWeights, mouth: 0 }
          : sample.channelWeights,
      })),
    );
    const targetPose = composed.rotations;
    for (const [expression, value] of composed.expressions) {
      if (this.speech?.playing && isMouthExpression(expression)) continue;
      this.setExpression(expression, value, "behavior");
    }
    const lookAt = this.speech?.playing ? undefined : composed.lookAt;
    const transitionSignature = `${this.motionSetRevision}:${samples
      .map((sample) => sample.entry.id)
      .join("|")}`;
    if (transitionSignature !== this.motionTransitionSignature) {
      this.transitionInertializer.capture(previousVisualPose, targetPose, visualAngularVelocity);
      this.motionTransitionSignature = transitionSignature;
    }
    const resolved = this.transitionInertializer.apply(targetPose, deltaSeconds);
    for (const [name, rotation] of resolved) instance.bones.get(name)?.quaternion.copy(rotation);
    const hips = instance.bones.get("hips");
    const restHips = instance.restPositions.get("hips");
    if (hips && restHips) {
      hips.position.copy(restHips);
      if (composed.hipsPosition) hips.position.add(composed.hipsPosition);
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
    return {
      ...(lookAt ? { lookAt } : {}),
    };
  }

  private collectMotionSamples(instance: AvatarInstance, now: number): WeightedMotionSample[] {
    const result: WeightedMotionSample[] = [];
    const add = (
      clip: ActiveMotionClip | undefined,
      priority: number,
      channelWeights: MotionChannelWeights,
      fixedWeight = 1,
    ) => {
      if (!clip?.ready || !this.isMotionEnabled(clip.id)) return;
      const entry = this.motionEntries.get(clip.id);
      if (!entry) return;
      let timeMs = Math.max(now - clip.startedAt, 0);
      if (clip === this.interactionMotion && this.headPatActive && entry.durationMs > 1) {
        timeMs %= entry.durationMs;
      }
      if (clip === this.speechMotion && this.speech) {
        timeMs = this.sampleSpeechPosition(now) ?? timeMs;
        if (entry.durationMs > 1) timeMs %= entry.durationMs;
      }
      const pose = this.motionLibrary.sample(instance.vrm, clip.id, timeMs, clip.mirror);
      if (!pose) return;
      result.push({
        clip,
        entry,
        pose,
        weight: fixedWeight * motionEnvelope(entry, timeMs),
        priority: clip.priority ?? priority,
        channelWeights: clip.channelWeights ?? channelWeights,
      });
    };
    // The base idle is always sampled underneath entrance and foreground clips.
    // Keeping it warm prevents empty composition frames from exposing the VRM rest/T pose.
    add(this.idleMotion, 0, channelsForFullBody());
    if (!this.dragging) {
      add(this.ambientMotion, 10, channelsForFullBody());
      add(this.entranceMotion, 20, channelsForFullBody());
      add(this.locomotionMotion, 30, channelsForFullBody(false));
      add(this.speechMotion, 40, channelsForSpeechBody(), 0.82);
      add(this.interactionMotion, 60, channelsForFullBody());
      for (const clip of this.requestedMotions.values()) {
        add(clip, 50, channelsForFullBody());
      }
    }
    return result;
  }

  private expireFinishedMotions(now: number): void {
    const expired = (clip: ActiveMotionClip | undefined, forceSinglePass = false) => {
      if (!clip?.ready) return false;
      const entry = this.motionEntries.get(clip.id);
      return Boolean(
        entry &&
        (forceSinglePass || entry.playbackMode === "once") &&
        !(clip === this.interactionMotion && this.headPatActive) &&
        now - clip.startedAt >= entry.durationMs,
      );
    };
    if (expired(this.entranceMotion, true)) {
      this.entranceMotion = undefined;
      this.startupSequenceComplete = true;
      this.ensureCoolIdle(now);
      this.nextAmbientMotionAt = now + AMBIENT_MOTION_INTERVAL_MS;
      this.motionSetRevision += 1;
    }
    if (expired(this.ambientMotion, true)) {
      this.ambientMotion = undefined;
      this.motionSetRevision += 1;
    }
    if (expired(this.interactionMotion, true)) {
      this.interactionMotion = undefined;
      this.motionSetRevision += 1;
    }
    let requestedExpired = false;
    for (const [requestId, clip] of this.requestedMotions) {
      if (!expired(clip)) continue;
      this.requestedMotions.delete(requestId);
      requestedExpired = true;
    }
    if (requestedExpired) this.motionSetRevision += 1;
  }

  private applyContinuousCorrections(instance: AvatarInstance): void {
    if (this.headPatActive) {
      const pressure = MathUtils.clamp(this.headPatPressure, 0, 1);
      this.applyBoneEuler(
        instance,
        "head",
        pressure * 0.055,
        0,
        this.headPatDirection * pressure * 0.075,
      );
      this.applyBoneEuler(instance, "chest", pressure * 0.018, 0, -this.headPatDirection * 0.025);
      this.setExpression("happy", 0.28 + pressure * 0.32, "reaction");
    }
    if (this.dragging) {
      this.applyBoneEuler(instance, "hips", 0, 0, -this.dragVelocity.x * 0.13);
      this.applyBoneEuler(
        instance,
        "chest",
        this.dragVelocity.y * 0.09,
        0,
        -this.dragVelocity.x * 0.08,
      );
      this.applyBoneEuler(
        instance,
        "head",
        -this.dragVelocity.y * 0.055,
        0,
        this.dragVelocity.x * 0.04,
      );
    }
    const correction = this.interactionCorrection;
    if (correction) {
      const elapsed = performance.now() - correction.startedAt;
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
      this.idleReturnReadyAt = now + IDLE_RETURN_SETTLE_MS;
      this.nextAmbientMotionAt = Math.max(
        this.nextAmbientMotionAt,
        now + AMBIENT_MOTION_INTERVAL_MS,
      );
    }
    this.foregroundWasActive = foregroundActive;

    if (
      this.pendingInteraction &&
      canStartQueuedInteraction(
        now,
        this.pendingInteraction.executeAt,
        this.idleReturnReadyAt,
        foregroundActive,
      )
    ) {
      const interaction = this.pendingInteraction;
      this.pendingInteraction = undefined;
      if (this.startInteraction(interaction.region, interaction.direction, now)) {
        foregroundActive = true;
        this.foregroundWasActive = true;
      }
    }

    if (
      !this.pendingInteraction &&
      !foregroundActive &&
      now >= this.idleReturnReadyAt &&
      now >= this.nextAmbientMotionAt
    ) {
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
      this.speechMotion ||
      this.speech?.playing ||
      this.locomotionMotion ||
      this.requestedMotions.size > 0,
    );
  }

  private startRandomAmbientMotion(now: number): void {
    const allCandidates = this.motionCatalog.entries.filter(
      (entry) =>
        ["idle", "gesture", "reaction"].includes(entry.category) &&
        this.isMotionEnabled(entry.id) &&
        /openmaiwaifu/i.test(entry.sourceProject) &&
        entry.rootMode !== "stage" &&
        !/(?:shy|cool).?waiting|appearing|talking|phone|sing|liked/i.test(entry.name),
    );
    const energy = this.speech?.playing ? 0.9 : this.listening ? 0.7 : 0.42;
    const entry = selectAmbientIdle(allCandidates, this.lastAmbientMotionId, energy, Math.random());
    this.nextAmbientMotionAt = now + AMBIENT_MOTION_INTERVAL_MS;
    if (!entry) return;
    this.lastAmbientMotionId = entry.id;
    this.ambientMotion = {
      id: entry.id,
      startedAt: now,
      mirror: false,
      ready: false,
    };
    void this.prepareClip(this.ambientMotion);
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
    this.interactionMotion = {
      id,
      startedAt: now,
      mirror: binding.mirrorBySide && entry.mirrorable && isLeftRegion(region, direction),
      ready: false,
    };
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
            entry.category === "speech" &&
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
    this.speechMotion = { id: entry.id, startedAt: now, mirror: false, ready: false };
    void this.prepareClip(this.speechMotion, playbackId);
    this.motionSetRevision += 1;
  }

  private updateLocomotionMotion(phase: StageLocomotionPhase, facing: -1 | 1, now: number): void {
    if (phase === "idle") {
      if (this.locomotionMotion) {
        this.locomotionMotion = undefined;
        this.motionSetRevision += 1;
      }
      return;
    }
    const entry = selectMotionForIntent(
      this.motionCatalog.entries.filter((value) => this.isMotionEnabled(value.id)),
      {
        category: "locomotion",
        tags: ["walk"],
        preferFingerMotion: true,
        preferredSource: "OpenMaiWaifu",
      },
    );
    if (!entry) return;
    const mirror = facing < 0 && entry.mirrorable;
    if (this.locomotionMotion?.id === entry.id) {
      if (this.locomotionMotion.mirror !== mirror) {
        this.locomotionMotion.mirror = mirror;
        this.motionSetRevision += 1;
      }
      return;
    }
    this.locomotionMotion = {
      id: entry.id,
      startedAt: now,
      mirror,
      ready: false,
      priority: 30,
      channelWeights: channelsForFullBody(false),
    };
    void this.prepareClip(this.locomotionMotion);
    this.motionSetRevision += 1;
  }

  private ensureCoolIdle(now: number, prepare = true): ActiveMotionClip | undefined {
    const candidates = this.motionCatalog.entries.filter(
      (entry) =>
        entry.category === "idle" &&
        entry.playbackMode === "loop" &&
        this.isMotionEnabled(entry.id) &&
        /openmaiwaifu/i.test(entry.sourceProject),
    );
    const currentAvailable = candidates.find((entry) => entry.id === this.idleMotion?.id);
    if (currentAvailable && /cool.?waiting/i.test(currentAvailable.name)) {
      if (prepare && this.idleMotion && !this.idleMotion.ready) {
        void this.prepareClip(this.idleMotion);
      }
      return this.idleMotion;
    }
    const entry = selectCoolIdle(candidates);
    if (!entry) return undefined;
    if (this.idleMotion?.id === entry.id) return this.idleMotion;
    const replacement: ActiveMotionClip = {
      id: entry.id,
      startedAt: now,
      mirror: false,
      ready: false,
    };
    this.idleMotion = replacement;
    if (prepare) void this.prepareClip(this.idleMotion);
    this.motionSetRevision += 1;
    return this.idleMotion;
  }

  private ensureStartupSequence(now: number): void {
    if (this.startupSequenceComplete) {
      this.ensureCoolIdle(now);
      return;
    }
    if (this.entranceMotion) {
      if (!this.entranceMotion.ready) void this.prepareClip(this.entranceMotion);
      return;
    }
    const entry = this.motionCatalog.entries.find(
      (candidate) =>
        candidate.name.trim().toLowerCase() === "appearing" && this.isMotionEnabled(candidate.id),
    );
    if (!entry) {
      if (this.motionCatalog.entries.length === 0) return;
      this.startupSequenceComplete = true;
      this.ensureCoolIdle(now);
      return;
    }
    this.entranceMotion = {
      id: entry.id,
      startedAt: now,
      mirror: false,
      ready: false,
    };
    void this.prepareClip(this.entranceMotion);
    // Keep the prepared base idle under the entrance so loading and fade-out
    // frames never fall through to the normalized VRM rest pose.
    this.ensureCoolIdle(now);
    this.motionSetRevision += 1;
  }

  private prepareClip(clip: ActiveMotionClip, playbackId?: string): Promise<boolean> {
    const instance = this.instance;
    if (!instance) return Promise.resolve(false);
    return this.motionLibrary
      .prepare(instance.vrm, clip.id)
      .then(() => {
        if (this.disposed || this.instance !== instance) return false;
        const stillCurrent =
          this.idleMotion === clip ||
          this.entranceMotion === clip ||
          this.ambientMotion === clip ||
          this.interactionMotion === clip ||
          this.locomotionMotion === clip ||
          (clip.requestId !== undefined && this.requestedMotions.get(clip.requestId) === clip) ||
          (this.speechMotion === clip && (!playbackId || this.speech?.playbackId === playbackId));
        if (!stillCurrent || !this.isMotionEnabled(clip.id)) return false;
        clip.ready = true;
        clip.startedAt = performance.now();
        this.motionSetRevision += 1;
        return true;
      })
      .catch((error: unknown) => {
        console.error(`Unable to prepare motion ${clip.id}`, error);
        if (this.disposed || this.instance !== instance) return false;
        if (this.entranceMotion === clip) {
          this.entranceMotion = undefined;
          this.startupSequenceComplete = true;
          const now = performance.now();
          this.ensureCoolIdle(now);
          this.nextAmbientMotionAt = now + AMBIENT_MOTION_INTERVAL_MS;
          this.motionSetRevision += 1;
        }
        return false;
      });
  }

  private preloadCatalogDefaults(vrm: VRM): void {
    const ids = new Set<string>();
    if (this.idleMotion) ids.add(this.idleMotion.id);
    if (this.entranceMotion) ids.add(this.entranceMotion.id);
    for (const binding of this.motionCatalog.bindings) {
      if (this.isMotionEnabled(binding.motionId)) ids.add(binding.motionId);
    }
    for (const entry of this.motionCatalog.entries
      .filter((value) => value.category === "speech" && this.isMotionEnabled(value.id))
      .slice(0, 3)) {
      ids.add(entry.id);
    }
    void this.motionLibrary
      .preload([...ids])
      .catch((error: unknown) => console.error("Unable to preload avatar motions", error));
    void Promise.all([...ids].map((id) => this.motionLibrary.prepare(vrm, id))).catch(
      (error: unknown) => console.error("Unable to compile preloaded avatar motions", error),
    );
  }

  private isMotionEnabled(id: string): boolean {
    return !this.disabledMotionIds.has(id);
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

export function capturePresentationRootBaseline(root: Object3D): PresentationRootBaseline {
  return {
    position: root.position.clone(),
    quaternion: root.quaternion.clone(),
    scale: root.scale.clone(),
  };
}

export function restorePresentationRoot(root: Object3D, baseline: PresentationRootBaseline): void {
  root.position.copy(baseline.position);
  root.quaternion.copy(baseline.quaternion);
  root.scale.copy(baseline.scale);
}

function scaleRatioWithinBounds(value: number, base: number): boolean {
  const ratio = Math.abs(value) / Math.max(Math.abs(base), 0.000_001);
  return ratio >= 0.75 && ratio <= 1.25;
}

function finiteNumber(value: number | null | undefined, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

/**
 * Fails closed when a behavior produces a non-finite or implausibly large presentation transform.
 * Normal actions stay within a small fraction of the avatar height, so exceeding these bounds means
 * the runtime has drifted or received corrupt motion data and should return to the captured baseline.
 */
export function stabilizePresentationRoot(
  root: Object3D,
  baseline: PresentationRootBaseline,
  avatarHeight: number,
): boolean {
  const maximumOffset = Math.max(Math.abs(avatarHeight), 0.001) * 0.35;
  const offsetX = root.position.x - baseline.position.x;
  const offsetY = root.position.y - baseline.position.y;
  const offsetZ = root.position.z - baseline.position.z;
  const offsetSquared = offsetX * offsetX + offsetY * offsetY + offsetZ * offsetZ;
  const finitePosition =
    Number.isFinite(root.position.x) &&
    Number.isFinite(root.position.y) &&
    Number.isFinite(root.position.z);
  const finiteQuaternion =
    Number.isFinite(root.quaternion.x) &&
    Number.isFinite(root.quaternion.y) &&
    Number.isFinite(root.quaternion.z) &&
    Number.isFinite(root.quaternion.w);
  const finiteScale =
    Number.isFinite(root.scale.x) && Number.isFinite(root.scale.y) && Number.isFinite(root.scale.z);
  const scaleWithinBounds =
    scaleRatioWithinBounds(root.scale.x, baseline.scale.x) &&
    scaleRatioWithinBounds(root.scale.y, baseline.scale.y) &&
    scaleRatioWithinBounds(root.scale.z, baseline.scale.z);
  const valid =
    finitePosition &&
    finiteQuaternion &&
    finiteScale &&
    offsetSquared <= maximumOffset * maximumOffset &&
    root.quaternion.lengthSq() >= 0.5 &&
    root.quaternion.lengthSq() <= 1.5 &&
    scaleWithinBounds;
  if (!valid) restorePresentationRoot(root, baseline);
  return valid;
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

function profileSoleOffsets(profile: AvatarAdaptationProfile): FootSoleOffsets {
  const offset = (id: string): Vector3 | undefined => {
    const contact = profile.contacts?.find((value) => value.id === id);
    if (!contact) return undefined;
    return new Vector3(
      finiteNumber(contact.localPosition[0], 0),
      finiteNumber(contact.localPosition[1], 0),
      finiteNumber(contact.localPosition[2], 0),
    );
  };
  return { left: offset("left_sole"), right: offset("right_sole") };
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

function isLeftRegion(region: InteractionRegion, direction: -1 | 1): boolean {
  return region.startsWith("left_") || (!region.startsWith("right_") && direction < 0);
}

function motionEnvelope(entry: MotionCatalogEntry, timeMs: number): number {
  const fadeIn = smooth01(timeMs / Math.max(entry.transitionInMs, 1));
  if (entry.playbackMode !== "once") return fadeIn;
  const remaining = entry.durationMs - timeMs;
  return Math.min(fadeIn, smooth01(remaining / Math.max(entry.transitionOutMs, 1)));
}

function smooth01(value: number): number {
  const clamped = Math.min(Math.max(value, 0), 1);
  return clamped * clamped * (3 - 2 * clamped);
}

function isMouthExpression(expression: string): boolean {
  return ["aa", "ih", "ou", "ee", "oh"].includes(expression.toLowerCase());
}

function disposeAvatarInstance(instance: AvatarInstance): void {
  instance.vrm?.springBoneManager?.reset();
  deepDisposeAvatarRoot(instance.presentationRoot);
}
