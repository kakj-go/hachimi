import type { MotionIntentRequest, MotionTransitionProfile } from "@hachimi/contracts";
import {
  type AnimationGraphSubmitOptions,
  channelWeightsFromIntent,
  type AnimationGraphLayer,
  type AnimationGraphNode,
  type MotionFeatureFrame,
  type MotionFeatureIndex,
  type MotionGraphCatalog,
  type PoseSampler,
} from "./types";
import { TransitionPlanner } from "./transition-planner";

const SLOT_ORDER = ["base", "locomotion", "speech", "action"] as const;

/** Deterministic, one-winner-per-slot graph. Product events only submit intents to this boundary. */
export class AnimationGraph {
  private readonly entries = new Map();
  private readonly profiles = new Map();
  private readonly features = new Map<string, MotionFeatureIndex>();
  private readonly nodes = new Map<string, AnimationGraphNode>();
  private readonly activeBySlot = new Map<(typeof SLOT_ORDER)[number], string>();
  private readonly planner = new TransitionPlanner();

  constructor(
    catalog: MotionGraphCatalog,
    private readonly sample: PoseSampler,
  ) {
    this.setCatalog(catalog);
  }

  setCatalog(catalog: MotionGraphCatalog): void {
    this.entries.clear();
    this.profiles.clear();
    for (const entry of catalog.entries) this.entries.set(entry.id, entry);
    for (const profile of catalog.transitionProfiles) this.profiles.set(profile.id, profile);
    for (const [id, node] of this.nodes)
      if (!this.entries.has(node.entry.id)) this.nodes.delete(id);
  }

  setFeatureIndex(index: MotionFeatureIndex): void {
    this.features.set(index.motionId, index);
  }

  submit(intent: MotionIntentRequest, nowMs: number, source?: MotionFeatureFrame): boolean {
    if (!intent.requestId.trim()) return false;
    if (!intent.active) {
      for (const [slot, requestId] of this.activeBySlot) {
        if (requestId === intent.requestId) this.activeBySlot.delete(slot);
      }
      return this.nodes.delete(intent.requestId);
    }
    const entry = this.entries.get(intent.motionId);
    if (!entry || entry.slot !== intent.slot) return false;
    const profile = this.profiles.get(entry.transitionProfileId);
    if (!profile) return false;
    const target = this.features.get(entry.id);
    const waitMs = this.transitionWaitMs(intent, nowMs, 120);
    const plan = target
      ? this.planner.plan(source, target, profile, intent.interruptPolicy)
      : {
          targetTimeMs: 0,
          durationMs: profile.maximumDurationMs,
          forced: true,
          cost: 1,
        };
    this.nodes.set(intent.requestId, {
      intent,
      entry,
      profile,
      startedAt: nowMs + waitMs,
      transitionStartedAt: nowMs + waitMs,
      activateAt: nowMs + waitMs,
      playbackTimeMs: plan.targetTimeMs,
      lastUpdatedAt: nowMs + waitMs,
      targetStartTimeMs: plan.targetTimeMs,
      transitionDurationMs: plan.durationMs,
      forced: plan.forced,
    });
    return true;
  }

  submitWithOptions(
    intent: MotionIntentRequest,
    nowMs: number,
    source?: MotionFeatureFrame,
    options: AnimationGraphSubmitOptions = {},
  ): boolean {
    const accepted = this.submit(intent, nowMs, source);
    if (!accepted || !intent.active) return accepted;
    const node = this.nodes.get(intent.requestId);
    if (!node) return false;
    const maximumWaitMs = options.maximumWaitMs ?? 120;
    const waitMs = this.transitionWaitMs(intent, nowMs, maximumWaitMs);
    node.startedAt = nowMs + waitMs;
    node.activateAt = nowMs + waitMs;
    node.transitionStartedAt =
      node.activateAt - Math.max(options.transitionElapsedMs ?? 0, 0);
    node.lastUpdatedAt = node.activateAt;
    return true;
  }

  updateIntent(intent: MotionIntentRequest): boolean {
    const node = this.nodes.get(intent.requestId);
    if (!node || node.entry.id !== intent.motionId || node.intent.slot !== intent.slot) return false;
    node.intent = intent;
    return true;
  }

  update(nowMs: number): AnimationGraphLayer[] {
    const layers: AnimationGraphLayer[] = [];
    for (const slot of SLOT_ORDER) {
      const candidates = [...this.nodes.values()].filter(
        (candidate) =>
          candidate.intent.slot === slot &&
          candidate.intent.active &&
          nowMs >= candidate.activateAt,
      );
      for (const candidate of candidates) {
        this.advancePlayback(candidate, nowMs);
        const localTime = candidate.playbackTimeMs;
        if (candidate.entry.loopMode !== "once" || localTime < candidate.entry.durationMs) continue;
        this.nodes.delete(candidate.intent.requestId);
        if (this.activeBySlot.get(slot) === candidate.intent.requestId) {
          this.activeBySlot.delete(slot);
        }
      }
      const liveCandidates = candidates
        .filter((candidate) => this.nodes.has(candidate.intent.requestId))
        .sort(
          (left, right) =>
            right.intent.priority - left.intent.priority ||
            right.startedAt - left.startedAt ||
            left.intent.requestId.localeCompare(right.intent.requestId),
        );
      const active = this.nodes.get(this.activeBySlot.get(slot) ?? "");
      const node =
        active?.intent.slot === slot && active.intent.interruptPolicy === "finish"
          ? active
          : liveCandidates[0];
      if (!node) continue;
      this.activeBySlot.set(slot, node.intent.requestId);
      this.advancePlayback(node, nowMs);
      const localTime = node.playbackTimeMs;
      const transitionElapsed = Math.max(nowMs - node.transitionStartedAt, 0);
      const transitionWeight = Math.min(
        transitionElapsed / Math.max(node.transitionDurationMs, 1),
        1,
      );
      layers.push({
        id: node.intent.requestId,
        motionId: node.entry.id,
        slot,
        priority: node.intent.priority,
        pose: this.sample(node.entry.id, localTime, node.intent.mirror && node.entry.mirrorable),
        weight: blendWeight(node.profile.blendProfileId, transitionWeight),
        channels: channelWeightsFromIntent(
          node.intent.channelWeights,
          effectiveChannelMask(node.entry.channelMask, node.profile.channelMask),
        ),
        inertialHalfLives: profileHalfLives(node.profile),
      });
    }
    return layers;
  }

  clear(): void {
    this.nodes.clear();
    this.features.clear();
    this.activeBySlot.clear();
  }

  has(requestId: string): boolean {
    return this.nodes.has(requestId);
  }

  safeSlots(nowMs: number): ReadonlySet<(typeof SLOT_ORDER)[number]> {
    const safe = new Set<(typeof SLOT_ORDER)[number]>();
    for (const slot of SLOT_ORDER) {
      const node = this.nodes.get(this.activeBySlot.get(slot) ?? "");
      if (!node || node.intent.interruptPolicy === "immediate") {
        safe.add(slot);
        continue;
      }
      const index = this.features.get(node.entry.id);
      if (!index) continue;
      this.advancePlayback(node, nowMs);
      const localTime = node.playbackTimeMs;
      const frame = index.frames[Math.min(Math.round((localTime / 1_000) * index.sampleHz), index.frames.length - 1)];
      if (frame?.safeExit) safe.add(slot);
    }
    return safe;
  }

  private transitionWaitMs(
    intent: MotionIntentRequest,
    nowMs: number,
    maximumWaitMs: number,
  ): number {
    if (intent.interruptPolicy !== "safe_point") return 0;
    const active = this.nodes.get(this.activeBySlot.get(intent.slot) ?? "");
    if (!active || active.intent.requestId === intent.requestId) return 0;
    const index = this.features.get(active.entry.id);
    if (!index) return Math.max(maximumWaitMs, 0);
    this.advancePlayback(active, nowMs);
    const localTime = active.playbackTimeMs;
    const frameIndex = Math.max(Math.round((localTime / 1_000) * index.sampleHz), 0);
    const maximumFrames = Math.ceil((Math.max(maximumWaitMs, 0) / 1_000) * index.sampleHz);
    for (let offset = 0; offset <= maximumFrames; offset += 1) {
      const frame = index.frames[Math.min(frameIndex + offset, index.frames.length - 1)];
      if (frame?.safeExit) return (offset / index.sampleHz) * 1_000;
    }
    return Math.max(maximumWaitMs, 0);
  }

  private advancePlayback(node: AnimationGraphNode, nowMs: number): void {
    if (nowMs <= node.lastUpdatedAt || nowMs < node.activateAt) return;
    const elapsed = nowMs - node.lastUpdatedAt;
    const speed = node.intent.locomotion?.desiredSpeed;
    const playbackRate =
      speed == null ? 1 : Math.min(Math.max(Math.abs(speed) / 0.28, 0.65), 1.25);
    node.playbackTimeMs += elapsed * playbackRate;
    node.lastUpdatedAt = nowMs;
  }
}

function smoothstep(value: number): number {
  return value * value * (3 - 2 * value);
}

function blendWeight(profileId: string, value: number): number {
  if (profileId.includes("linear")) return value;
  if (profileId.includes("ease_out")) return 1 - (1 - value) * (1 - value);
  return smoothstep(value);
}

function profileHalfLives(profile: MotionTransitionProfile) {
  const configured = profile.inertialHalfLives;
  return {
    root: (configured?.rootMs ?? 100) / 1_000,
    body: (configured?.bodyMs ?? 80) / 1_000,
    arms: (configured?.armsMs ?? 65) / 1_000,
    lookAt: (configured?.lookAtMs ?? 60) / 1_000,
    expression: (configured?.expressionMs ?? 50) / 1_000,
  };
}

function effectiveChannelMask(
  entry: readonly import("@hachimi/contracts").BehaviorChannel[],
  profile: readonly import("@hachimi/contracts").BehaviorChannel[],
) {
  if (profile.includes("full_body")) return [...entry];
  if (entry.includes("full_body")) return [...profile];
  return entry.filter((channel) => profile.includes(channel));
}
