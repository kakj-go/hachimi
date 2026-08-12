import {
  commands,
  type AvatarRuntimeAsset,
  type MotionCatalogEntry,
  type MotionTransitionProfile,
} from "@hachimi/contracts";
import {
  AvatarConstraintPipeline,
  FootContactAnalyzer,
  FullPoseInertializer,
  MotionAssetLibrary,
  VRM_HUMANOID_BONES,
  runtimeBoneName,
  measureMotionPoseSeam,
  nearestFeatureFrame,
  TransitionPlanner,
  deepDisposeAvatarRoot,
  loadAvatarWithDomTextures,
  limitPoseStep,
  type AvatarConstraintDiagnostics,
  type FootSoleOffsets,
  type MotionFeatureFrame,
} from "@hachimi/avatar-motion-runtime";
import { VRMLoaderPlugin, VRMUtils, type VRM } from "@pixiv/three-vrm";
import { VRMAnimationLoaderPlugin } from "@pixiv/three-vrm-animation";
import {
  AmbientLight,
  Box3,
  BufferAttribute,
  BufferGeometry,
  Clock,
  DirectionalLight,
  GridHelper,
  Group,
  HemisphereLight,
  Line,
  LineBasicMaterial,
  MathUtils,
  NeutralToneMapping,
  PerspectiveCamera,
  Scene,
  SkeletonHelper,
  SRGBColorSpace,
  Vector3,
  WebGLRenderer,
  type Object3D,
  type Quaternion,
} from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";

export interface MotionLabFrame {
  timeMs: number;
  phase: number;
  activeBones: number;
  fingerBones: number;
  maxAngleDegrees: number;
  collisionCount: number;
  maxFootDriftNormalized: number;
  groundPenetrationNormalized: number;
  maxJointCorrectionDegrees: number;
  solveTimeMs: number;
  compiledCacheSize: number;
  activeBoneNames: readonly string[];
  rootPosition: readonly [number, number, number];
  rootDistance: number;
  loopSeamDegrees: number;
  loopSeamRootDistance: number;
  leftFootPhase: string;
  rightFootPhase: string;
  contactTimeline: string;
}

export interface MotionTransitionDiagnostic {
  sourceTimeMs: number;
  entryTimeMs: number;
  durationMs: number;
  forced: boolean;
  totalCost: number;
  poseCost: number;
  velocityCost: number;
  footContactCost: number;
  rootCost: number;
  sourceFootContact: string;
  targetFootContact: string;
  sourceBoneCount: number;
  targetBoneCount: number;
  peakBoneStepDegrees: number;
  peakRootStepNormalized: number;
  peakLookAtStepDegrees: number;
  accepted: boolean;
}

export interface MotionTransitionMatrixCell extends MotionTransitionDiagnostic {
  sourceMotionId: string;
  targetMotionId: string;
}

type FrameListener = (frame: MotionLabFrame) => void;
export type MotionRuntimeVisualMode = "preview" | "diagnostics";

export class MotionLabRuntime {
  private readonly renderer: WebGLRenderer;
  private readonly scene = new Scene();
  private readonly camera = new PerspectiveCamera(30, 1, 0.01, 100);
  private readonly loader = new GLTFLoader();
  private readonly motionLoader = new GLTFLoader();
  private readonly motionLibrary: MotionAssetLibrary;
  private readonly constraints = new AvatarConstraintPipeline();
  private readonly footContacts = new FootContactAnalyzer();
  private readonly transitionPlanner = new TransitionPlanner();
  private readonly transitionProfiles = new Map<string, MotionTransitionProfile>();
  private readonly clock = new Clock();
  private readonly resizeObserver: ResizeObserver;
  private frameRequest = 0;
  private root: Group | undefined;
  private vrm: VRM | undefined;
  private skeleton: SkeletonHelper | undefined;
  private profile: AvatarRuntimeAsset["profile"] | undefined;
  private avatarHeight = 1.6;
  private readonly bones = new Map<string, Object3D>();
  private readonly restRotations = new Map<string, Quaternion>();
  private readonly restPositions = new Map<string, Vector3>();
  private motion: MotionCatalogEntry | undefined;
  private motionReady = false;
  private timeMs = 0;
  private playing = true;
  private speed = 1;
  private mirror = false;
  private avatarLoadGeneration = 0;
  private frameListener: FrameListener | undefined;
  private diagnostics = emptyConstraintDiagnostics();
  private readonly rootTrail: Vector3[] = [];
  private readonly contactHistory: string[] = [];
  private readonly rootTrailGeometry = new BufferGeometry();
  private readonly rootTrailPositions = new Float32Array(240 * 3);
  private readonly rootTrailLine = new Line(
    this.rootTrailGeometry,
    new LineBasicMaterial({ color: 0x6d8dff }),
  );

  constructor(
    private readonly container: HTMLElement,
    private readonly options: { visualMode?: MotionRuntimeVisualMode } = {},
  ) {
    this.renderer = new WebGLRenderer({ antialias: true, alpha: true, premultipliedAlpha: true });
    this.renderer.outputColorSpace = SRGBColorSpace;
    this.renderer.toneMapping = NeutralToneMapping;
    this.renderer.toneMappingExposure = 0.92;
    this.container.append(this.renderer.domElement);
    this.loader.register((parser) => new VRMLoaderPlugin(parser));
    this.motionLoader.register((parser) => new VRMAnimationLoaderPlugin(parser));
    this.motionLibrary = new MotionAssetLibrary(
      this.motionLoader,
      (id) => commands.getMotionRuntimeAsset(id),
      {
        read: (cacheKey) => commands.readMotionFeatureIndex({ cacheKey }),
        write: (cacheKey, payload) =>
          commands.writeMotionFeatureIndex({ cacheKey, payload }),
      },
    );
    this.scene.add(new AmbientLight(0xffffff, 0.7));
    this.scene.add(new HemisphereLight(0xf2f0ff, 0x554b68, 0.9));
    const key = new DirectionalLight(0xfff7f1, 1.65);
    key.position.set(3, 5, 5);
    this.scene.add(key);
    const fill = new DirectionalLight(0xb9ccff, 0.45);
    fill.position.set(-4, 2, 3);
    this.scene.add(fill);
    if (this.visualMode === "diagnostics") {
      this.scene.add(new GridHelper(4, 40, 0x777777, 0x333333));
    }
    this.rootTrailGeometry.setAttribute(
      "position",
      new BufferAttribute(this.rootTrailPositions, 3),
    );
    this.rootTrailGeometry.setDrawRange(0, 0);
    if (this.visualMode === "diagnostics") this.scene.add(this.rootTrailLine);
    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(container);
    this.resize();
    this.clock.start();
    this.frameRequest = requestAnimationFrame(this.animate);
  }

  private get visualMode(): MotionRuntimeVisualMode {
    return this.options.visualMode ?? "diagnostics";
  }

  setCatalog(
    entries: readonly MotionCatalogEntry[],
    profiles: readonly MotionTransitionProfile[] = [],
  ): void {
    this.motionLibrary.setCatalog(entries);
    this.transitionProfiles.clear();
    for (const profile of profiles) this.transitionProfiles.set(profile.id, profile);
  }

  async analyzeTransition(
    source: MotionCatalogEntry,
    target: MotionCatalogEntry,
  ): Promise<MotionTransitionDiagnostic> {
    if (!this.vrm) throw new Error("Load an avatar before transition analysis");
    const sourceProfile = this.transitionProfiles.get(source.transitionProfileId);
    const targetProfile = this.transitionProfiles.get(target.transitionProfileId);
    if (!sourceProfile || !targetProfile) throw new Error("Motion transition profile is missing");
    await Promise.all([
      this.motionLibrary.prepare(this.vrm, source.id),
      this.motionLibrary.prepare(this.vrm, target.id),
    ]);
    const [sourceIndex, targetIndex] = await Promise.all([
      this.motionLibrary.prepareFeatureIndex(this.vrm, source.id, sourceProfile),
      this.motionLibrary.prepareFeatureIndex(this.vrm, target.id, targetProfile),
    ]);
    if (!sourceIndex || !targetIndex) throw new Error("Motion feature analysis failed");
    const sourceFrame = nearestFeatureFrame(sourceIndex, this.timeMs);
    const plan = this.transitionPlanner.plan(
      sourceFrame,
      targetIndex,
      targetProfile,
      targetProfile.interruptPolicy,
    );
    const targetFrame = nearestFeatureFrame(targetIndex, plan.targetTimeMs);
    const peaks = simulateTransitionPeaks(
      sourceFrame,
      targetFrame,
      targetProfile,
      this.avatarHeight,
    );
    return {
      sourceTimeMs: sourceFrame.timeMs,
      entryTimeMs: plan.targetTimeMs,
      durationMs: plan.durationMs,
      forced: plan.forced,
      totalCost: plan.cost,
      poseCost: plan.costs.pose,
      velocityCost: plan.costs.velocity,
      footContactCost: plan.costs.footContact,
      rootCost: plan.costs.root,
      sourceFootContact: sourceFrame.footContact,
      targetFootContact: targetFrame.footContact,
      sourceBoneCount: sourceFrame.pose.rotations.size,
      targetBoneCount: targetFrame.pose.rotations.size,
      ...peaks,
      accepted: transitionPeaksAccepted(peaks),
    };
  }

  async analyzeTransitionMatrix(
    entries: readonly MotionCatalogEntry[],
    onProgress?: (completed: number, total: number) => void,
  ): Promise<MotionTransitionMatrixCell[]> {
    const cells: MotionTransitionMatrixCell[] = [];
    const total = entries.length * Math.max(entries.length - 1, 0);
    for (const source of entries) {
      for (const target of entries) {
        if (source.id === target.id) continue;
        cells.push({
          sourceMotionId: source.id,
          targetMotionId: target.id,
          ...(await this.analyzeTransition(source, target)),
        });
        onProgress?.(cells.length, total);
      }
    }
    return cells;
  }

  async loadAvatar(asset: AvatarRuntimeAsset): Promise<void> {
    const generation = ++this.avatarLoadGeneration;
    const gltf = await loadAvatarWithDomTextures(this.loader, asset.assetUrl);
    if (generation !== this.avatarLoadGeneration) {
      deepDisposeAvatarRoot(gltf.scene);
      return;
    }
    const vrm = gltf.userData["vrm"] as VRM | undefined;
    if (!vrm) throw new Error("Motion Lab requires a Runtime Ready VRM");
    if (asset.format === "vrm0") VRMUtils.rotateVRM0(vrm);
    if (this.root) {
      this.scene.remove(this.root);
      deepDisposeAvatarRoot(this.root);
    }
    if (this.vrm) this.motionLibrary.clear(this.vrm);
    const root = new Group();
    root.add(vrm.scene);
    const initialBounds = new Box3().setFromObject(root);
    const center = initialBounds.getCenter(new Vector3());
    vrm.scene.position.set(-center.x, -initialBounds.min.y, -center.z);
    this.root = root;
    this.vrm = vrm;
    this.profile = asset.profile;
    this.avatarHeight = Math.max(new Box3().setFromObject(root).getSize(new Vector3()).y, 0.1);
    this.bones.clear();
    this.restRotations.clear();
    this.restPositions.clear();
    for (const vrmName of VRM_HUMANOID_BONES) {
      const bone = vrm.humanoid.getNormalizedBoneNode(vrmName);
      if (!bone) continue;
      const name = runtimeBoneName(vrmName);
      this.bones.set(name, bone);
      this.restRotations.set(name, bone.quaternion.clone());
      this.restPositions.set(name, bone.position.clone());
    }
    this.skeleton = this.visualMode === "diagnostics" ? new SkeletonHelper(vrm.scene) : undefined;
    if (this.skeleton) root.add(this.skeleton);
    this.scene.add(root);
    this.frameAvatar(root);
    this.constraints.reset();
    this.footContacts.reset();
    this.rootTrail.length = 0;
    this.contactHistory.length = 0;
    if (this.motion) await this.prepareMotion(this.motion);
  }

  async setMotion(entry: MotionCatalogEntry): Promise<void> {
    this.motion = entry;
    this.timeMs = 0;
    this.rootTrail.length = 0;
    this.contactHistory.length = 0;
    this.rootTrailGeometry.setDrawRange(0, 0);
    await this.prepareMotion(entry);
  }

  clearMotion(): void {
    this.motion = undefined;
    this.motionReady = false;
    this.timeMs = 0;
  }

  private async prepareMotion(entry: MotionCatalogEntry): Promise<void> {
    const vrm = this.vrm;
    this.motionReady = false;
    if (!vrm) return;
    await this.motionLibrary.prepare(vrm, entry.id);
    if (this.vrm === vrm && this.motion?.id === entry.id) this.motionReady = true;
  }

  setPlaying(value: boolean): void {
    this.playing = value;
  }

  setSpeed(value: number): void {
    this.speed = MathUtils.clamp(value, 0.25, 3);
  }

  setMirror(value: boolean): void {
    this.mirror = value;
    this.restart();
  }

  restart(): void {
    this.timeMs = 0;
  }

  setTimeMs(value: number): void {
    this.timeMs = MathUtils.clamp(value, 0, this.motion?.durationMs ?? 0);
    this.rootTrail.length = 0;
    this.contactHistory.length = 0;
    this.rootTrailGeometry.setDrawRange(0, 0);
    this.footContacts.reset();
  }

  setFrameListener(listener: FrameListener): void {
    this.frameListener = listener;
  }

  dispose(): void {
    this.avatarLoadGeneration += 1;
    cancelAnimationFrame(this.frameRequest);
    this.resizeObserver.disconnect();
    if (this.vrm) this.motionLibrary.clear(this.vrm);
    if (this.root) deepDisposeAvatarRoot(this.root);
    this.rootTrailGeometry.dispose();
    this.rootTrailLine.material.dispose();
    this.renderer.dispose();
    this.renderer.domElement.remove();
  }

  private readonly animate = () => {
    this.frameRequest = requestAnimationFrame(this.animate);
    const delta = Math.min(this.clock.getDelta(), 0.05);
    const entry = this.motion;
    if (this.playing && entry) {
      this.timeMs += delta * 1_000 * this.speed;
      if (entry.loopMode === "loop" && entry.durationMs > 0) this.timeMs %= entry.durationMs;
      else this.timeMs = Math.min(this.timeMs, entry.durationMs);
    }
    const started = performance.now();
    let activeBones = 0;
    let activeBoneNames: readonly string[] = [];
    let maxAngleDegrees = 0;
    let rootPosition: [number, number, number] = [0, 0, 0];
    let rootDistance = 0;
    let loopSeamDegrees = 0;
    let loopSeamRootDistance = 0;
    let leftFootPhase = "air";
    let rightFootPhase = "air";
    if (this.vrm) {
      for (const [name, rest] of this.restRotations) this.bones.get(name)?.quaternion.copy(rest);
      for (const [name, rest] of this.restPositions) this.bones.get(name)?.position.copy(rest);
      this.vrm.expressionManager?.resetValues();
      const sample =
        entry && this.motionReady
          ? this.motionLibrary.sample(this.vrm, entry.id, this.timeMs, this.mirror)
          : undefined;
      if (sample) {
        activeBones = sample.rotations.size;
        activeBoneNames = [...sample.rotations.keys()].sort();
        for (const [name, rotation] of sample.rotations) {
          const bone = this.bones.get(name);
          const rest = this.restRotations.get(name);
          if (!bone) continue;
          bone.quaternion.copy(rotation);
          if (rest)
            maxAngleDegrees = Math.max(maxAngleDegrees, MathUtils.radToDeg(rest.angleTo(rotation)));
        }
        const hips = this.bones.get("hips");
        const restHips = this.restPositions.get("hips");
        if (hips && restHips && sample.hipsPosition)
          hips.position.copy(restHips).add(sample.hipsPosition);
        if (sample.hipsPosition) {
          rootPosition = sample.hipsPosition.toArray() as [number, number, number];
          rootDistance = Math.hypot(sample.hipsPosition.x, sample.hipsPosition.z);
          if (this.playing) this.rootTrail.push(sample.hipsPosition.clone());
          if (this.rootTrail.length > 240) this.rootTrail.shift();
          for (let index = 0; index < this.rootTrail.length; index += 1) {
            const point = this.rootTrail[index]!;
            this.rootTrailPositions[index * 3] = point.x;
            this.rootTrailPositions[index * 3 + 1] = point.y;
            this.rootTrailPositions[index * 3 + 2] = point.z;
          }
          this.rootTrailGeometry.attributes["position"]!.needsUpdate = true;
          this.rootTrailGeometry.setDrawRange(0, this.rootTrail.length);
        }
        for (const [expression, weight] of sample.expressions) {
          this.vrm.expressionManager?.setValue(expression, weight);
        }
        if (sample.lookAt && this.vrm.lookAt) {
          this.vrm.lookAt.yaw = sample.lookAt.yawDegrees;
          this.vrm.lookAt.pitch = sample.lookAt.pitchDegrees;
        }
        if (entry?.loopMode === "loop") {
          const start = this.motionLibrary.sample(this.vrm, entry.id, 0, this.mirror);
          const end = this.motionLibrary.sample(
            this.vrm,
            entry.id,
            entry.durationMs - 0.001,
            this.mirror,
          );
          if (start && end) {
            const seam = measureMotionPoseSeam(start, end);
            loopSeamDegrees = seam.maxRotationDegrees;
            loopSeamRootDistance = seam.rootDistance;
          }
        }
      }
      if (this.profile) {
        this.root?.updateWorldMatrix(true, true);
        const contact = this.footContacts.update(
          this.bones,
          this.avatarHeight,
          0,
          Math.max(delta, 1 / 240),
          profileSoleOffsets(this.profile),
        );
        leftFootPhase = contact.left.phase;
        rightFootPhase = contact.right.phase;
        if (this.playing) {
          this.contactHistory.push(`${phaseGlyph(leftFootPhase)}${phaseGlyph(rightFootPhase)}`);
        }
        if (this.contactHistory.length > 80) this.contactHistory.shift();
        this.diagnostics = this.constraints.solve(
          {
            bones: this.bones,
            restRotations: this.restRotations,
            profile: this.profile,
            height: this.avatarHeight,
            groundY: 0,
          },
          {
            leftFootPhase: contact.left.phase,
            rightFootPhase: contact.right.phase,
            footStrength: contact.left.phase === "air" && contact.right.phase === "air" ? 0 : 0.82,
            endEffectors: [],
            centerOfMass: contact.centerOfMass,
          },
        );
      }
      this.vrm.update(this.playing ? delta : 0);
    }
    this.renderer.render(this.scene, this.camera);
    this.frameListener?.({
      timeMs: this.timeMs,
      phase: entry ? this.timeMs / Math.max(entry.durationMs, 1) : 0,
      activeBones,
      fingerBones: entry?.fingerBoneCount ?? 0,
      maxAngleDegrees,
      collisionCount: this.diagnostics.collisionCount,
      maxFootDriftNormalized: this.diagnostics.maxFootDriftNormalized,
      groundPenetrationNormalized: this.diagnostics.groundPenetrationNormalized,
      maxJointCorrectionDegrees: this.diagnostics.maxJointCorrectionDegrees,
      solveTimeMs: performance.now() - started,
      compiledCacheSize: this.vrm ? this.motionLibrary.compiledCount(this.vrm) : 0,
      activeBoneNames,
      rootPosition,
      rootDistance,
      loopSeamDegrees,
      loopSeamRootDistance,
      leftFootPhase,
      rightFootPhase,
      contactTimeline: this.contactHistory.join(" "),
    });
  };

  private resize(): void {
    const width = Math.max(this.container.clientWidth, 1);
    const height = Math.max(this.container.clientHeight, 1);
    this.renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
  }

  private frameAvatar(root: Object3D): void {
    const size = new Box3().setFromObject(root).getSize(new Vector3());
    const targetY = size.y * 0.5;
    const distance = (size.y / (2 * Math.tan(MathUtils.degToRad(this.camera.fov / 2)))) * 1.18;
    this.camera.position.set(0, targetY, distance);
    this.camera.lookAt(0, targetY, 0);
    this.camera.updateProjectionMatrix();
  }
}

export function motionFrameHealthy(frame: MotionLabFrame): boolean {
  return Object.values(frame).every((value) => {
    if (typeof value === "number") return Number.isFinite(value);
    if (Array.isArray(value)) {
      return value.every((item) => typeof item !== "number" || Number.isFinite(item));
    }
    return true;
  });
}

export function transitionPeaksAccepted(peaks: {
  peakBoneStepDegrees: number;
  peakRootStepNormalized: number;
  peakLookAtStepDegrees: number;
}): boolean {
  return (
    Number.isFinite(peaks.peakBoneStepDegrees) &&
    Number.isFinite(peaks.peakRootStepNormalized) &&
    Number.isFinite(peaks.peakLookAtStepDegrees) &&
    peaks.peakBoneStepDegrees <= 12.001 &&
    peaks.peakRootStepNormalized <= 0.00501 &&
    peaks.peakLookAtStepDegrees <= 4.001
  );
}

function simulateTransitionPeaks(
  source: MotionFeatureFrame,
  target: MotionFeatureFrame,
  profile: MotionTransitionProfile,
  avatarHeight: number,
) {
  const inertializer = new FullPoseInertializer();
  inertializer.capture(source.pose, target.pose, source.velocity, target.velocity);
  const configured = profile.inertialHalfLives;
  const halfLives = {
    root: (configured?.rootMs ?? 100) / 1_000,
    body: (configured?.bodyMs ?? 80) / 1_000,
    arms: (configured?.armsMs ?? 65) / 1_000,
    lookAt: (configured?.lookAtMs ?? 60) / 1_000,
    expression: (configured?.expressionMs ?? 50) / 1_000,
  };
  let previous = source.pose;
  let peakBoneStepDegrees = 0;
  let peakRootStepNormalized = 0;
  let peakLookAtStepDegrees = 0;
  for (let frame = 0; frame < 30; frame += 1) {
    const current = inertializer.apply(target.pose, 1 / 60, halfLives);
    const step = limitPoseStep(current, previous, avatarHeight);
    peakBoneStepDegrees = Math.max(peakBoneStepDegrees, step.boneDegrees);
    peakRootStepNormalized = Math.max(peakRootStepNormalized, step.rootHeightRatio);
    peakLookAtStepDegrees = Math.max(peakLookAtStepDegrees, step.lookAtDegrees);
    previous = current;
  }
  return { peakBoneStepDegrees, peakRootStepNormalized, peakLookAtStepDegrees };
}

function emptyConstraintDiagnostics(): AvatarConstraintDiagnostics {
  return {
    leftFootPhase: "air",
    rightFootPhase: "air",
    maxFootDriftNormalized: 0,
    groundPenetrationNormalized: 0,
    maxJointCorrectionDegrees: 0,
    collisionCount: 0,
    centerOfMassOutsideSupport: false,
  };
}

function phaseGlyph(phase: string): string {
  return phase === "air" ? "·" : phase === "toe" ? "T" : phase === "heel" ? "H" : "F";
}

function profileSoleOffsets(profile: AvatarRuntimeAsset["profile"] | undefined): FootSoleOffsets {
  const find = (id: string): Vector3 | undefined => {
    const contact = profile?.contacts?.find((value) => value.id === id);
    if (!contact) return undefined;
    return new Vector3(
      contact.localPosition[0] ?? 0,
      contact.localPosition[1] ?? 0,
      contact.localPosition[2] ?? 0,
    );
  };
  return { left: find("left_sole"), right: find("right_sole") };
}
