import { MathUtils } from "three";

export interface FaceGazeFrame {
  eyeYaw: number;
  eyePitch: number;
  headYaw: number;
  headPitch: number;
  chestYaw: number;
  blink: number;
}

export type ExpressionLayer = "base" | "behavior" | "reaction" | "blink_viseme";

const EXPRESSION_LAYER_ORDER: readonly ExpressionLayer[] = [
  "base",
  "behavior",
  "reaction",
  "blink_viseme",
];
const EMOTIONS = new Set(["happy", "relaxed", "sad", "angry", "surprised"]);

export const CURSOR_GAZE_PROFILE = {
  yawDegrees: 32,
  pitchDegrees: 20,
  yawDeadZone: 0.04,
  pitchDeadZone: 0.04,
  attentionMs: 425,
  targetPitchMinDegrees: -18,
  eyeYawDegrees: 28,
  eyePitchMinDegrees: -14,
  eyePitchMaxDegrees: 18,
  headYawShare: 0.75,
  headPitchShare: 0.65,
  headFollowDelayMs: 40,
  headDamping: 10,
  chestThresholdDegrees: 14,
  chestYawShare: 0.25,
  chestFollowDelayMs: 120,
  chestDamping: 4.5,
} as const;

/**
 * Frame-local four-layer expression stack. VRM's ExpressionManager applies each expression's
 * overrideBlink/overrideLookAt/overrideMouth metadata after these semantic weights are resolved.
 */
export class FaceExpressionMixer {
  private readonly layers = new Map<ExpressionLayer, Map<string, number>>();

  beginFrame(): void {
    this.layers.clear();
  }

  set(layer: ExpressionLayer, expression: string, weight: number): void {
    let values = this.layers.get(layer);
    if (!values) {
      values = new Map();
      this.layers.set(layer, values);
    }
    values.set(expression, MathUtils.clamp(weight, 0, 1));
  }

  resolve(): ReadonlyMap<string, number> {
    const resolved = new Map<string, number>();
    for (const layer of EXPRESSION_LAYER_ORDER) {
      for (const [expression, weight] of this.layers.get(layer) ?? []) {
        // Higher semantic layers replace lower ones for the same expression. Different facial
        // domains remain composable and are finalized by the VRM override flags.
        resolved.set(expression, weight);
      }
    }
    const emotionWeight = [...resolved]
      .filter(([expression]) => EMOTIONS.has(expression))
      .reduce((sum, [, weight]) => sum + weight, 0);
    if (emotionWeight > 1) {
      for (const [expression, weight] of resolved) {
        if (EMOTIONS.has(expression)) resolved.set(expression, weight / emotionWeight);
      }
    }
    const mouth = ["aa", "ih", "ou", "ee", "oh"];
    const mouthWeight = mouth.reduce((sum, expression) => sum + (resolved.get(expression) ?? 0), 0);
    if (mouthWeight > 1) {
      for (const expression of mouth) {
        const weight = resolved.get(expression);
        if (weight !== undefined) resolved.set(expression, weight / mouthWeight);
      }
    }
    return resolved;
  }
}

export class FaceGazeRuntime {
  private nextBlinkAt = 0;
  private blinkStartedAt = -1;
  private secondBlinkAt = -1;
  private targetYaw = 0;
  private targetPitch = 0;
  private eyeYaw = 0;
  private eyePitch = 0;
  private headYaw = 0;
  private headPitch = 0;
  private chestYaw = 0;
  private attentionUntil = 0;
  private headFollowAfter = 0;
  private chestFollowAfter = 0;
  private speaking = false;
  private sleepiness = 0;
  private curiosity = 0.5;
  private energy = 0.75;

  constructor(private readonly random: () => number = Math.random) {}

  update(
    nowMs: number,
    deltaSeconds: number,
    attention?: { yaw: number; pitch: number },
  ): FaceGazeFrame {
    if (this.nextBlinkAt === 0) this.scheduleBlink(nowMs);
    if (nowMs >= this.nextBlinkAt) {
      this.blinkStartedAt = nowMs;
      const doubleBlink = this.random() < 0.12;
      this.secondBlinkAt = doubleBlink ? nowMs + 190 : -1;
      this.scheduleBlink(nowMs);
    }
    if (this.secondBlinkAt > 0 && nowMs >= this.secondBlinkAt) {
      this.blinkStartedAt = this.secondBlinkAt;
      this.secondBlinkAt = -1;
    }
    if (attention) {
      this.targetYaw = MathUtils.clamp(
        attention.yaw,
        -CURSOR_GAZE_PROFILE.yawDegrees,
        CURSOR_GAZE_PROFILE.yawDegrees,
      );
      this.targetPitch = MathUtils.clamp(
        attention.pitch,
        CURSOR_GAZE_PROFILE.targetPitchMinDegrees,
        CURSOR_GAZE_PROFILE.pitchDegrees,
      );
    } else if (
      nowMs >= this.attentionUntil &&
      this.random() <
        deltaSeconds *
          MathUtils.lerp(0.08, 0.36, this.curiosity) *
          MathUtils.lerp(0.72, 1.08, this.energy)
    ) {
      this.targetYaw = truncatedNormal(
        this.random,
        0,
        MathUtils.lerp(5, 10, this.curiosity),
        -20,
        20,
      );
      this.targetPitch = truncatedNormal(
        this.random,
        -1,
        MathUtils.lerp(3, 5, this.curiosity),
        -10,
        10,
      );
    }
    const eyeYawTarget = MathUtils.clamp(
      this.targetYaw,
      -CURSOR_GAZE_PROFILE.eyeYawDegrees,
      CURSOR_GAZE_PROFILE.eyeYawDegrees,
    );
    const eyePitchTarget = MathUtils.clamp(
      this.targetPitch,
      CURSOR_GAZE_PROFILE.eyePitchMinDegrees,
      CURSOR_GAZE_PROFILE.eyePitchMaxDegrees,
    );
    this.eyeYaw = damp(this.eyeYaw, eyeYawTarget, 18, deltaSeconds);
    this.eyePitch = damp(this.eyePitch, eyePitchTarget, 18, deltaSeconds);
    const headYawTarget =
      nowMs >= this.headFollowAfter
        ? this.targetYaw * CURSOR_GAZE_PROFILE.headYawShare
        : this.headYaw;
    const headPitchTarget =
      nowMs >= this.headFollowAfter
        ? this.targetPitch * CURSOR_GAZE_PROFILE.headPitchShare
        : this.headPitch;
    this.headYaw = damp(
      this.headYaw,
      headYawTarget,
      CURSOR_GAZE_PROFILE.headDamping,
      deltaSeconds,
    );
    this.headPitch = damp(
      this.headPitch,
      headPitchTarget,
      CURSOR_GAZE_PROFILE.headDamping,
      deltaSeconds,
    );
    const chestTarget =
      nowMs >= this.chestFollowAfter &&
      Math.abs(this.targetYaw) > CURSOR_GAZE_PROFILE.chestThresholdDegrees
        ? this.targetYaw * CURSOR_GAZE_PROFILE.chestYawShare
        : 0;
    this.chestYaw = damp(
      this.chestYaw,
      chestTarget,
      CURSOR_GAZE_PROFILE.chestDamping,
      deltaSeconds,
    );
    const blinkElapsed = nowMs - this.blinkStartedAt;
    const blink =
      blinkElapsed >= 0 && blinkElapsed <= 170 ? Math.sin((blinkElapsed / 170) * Math.PI) : 0;
    return {
      eyeYaw: this.eyeYaw,
      eyePitch: this.eyePitch,
      headYaw: this.headYaw,
      headPitch: this.headPitch,
      chestYaw: this.chestYaw,
      blink,
    };
  }

  attend(yaw: number, pitch: number, nowMs: number, durationMs = 900): void {
    const rapidShift =
      Math.abs(yaw - this.targetYaw) > 18 || Math.abs(pitch - this.targetPitch) > 10;
    const acquiringTarget = nowMs >= this.attentionUntil;
    this.targetYaw = MathUtils.clamp(
      yaw,
      -CURSOR_GAZE_PROFILE.yawDegrees,
      CURSOR_GAZE_PROFILE.yawDegrees,
    );
    this.targetPitch = MathUtils.clamp(
      pitch,
      CURSOR_GAZE_PROFILE.targetPitchMinDegrees,
      CURSOR_GAZE_PROFILE.pitchDegrees,
    );
    this.attentionUntil = nowMs + Math.max(durationMs, 0);
    if (rapidShift || acquiringTarget) {
      this.headFollowAfter = nowMs + CURSOR_GAZE_PROFILE.headFollowDelayMs;
      this.chestFollowAfter = nowMs + CURSOR_GAZE_PROFILE.chestFollowDelayMs;
    }
    if (rapidShift)
      this.nextBlinkAt = Math.min(this.nextBlinkAt || Number.POSITIVE_INFINITY, nowMs + 90);
  }

  attendCursor(rawYaw: number, rawPitch: number, nowMs: number): void {
    const yaw = Math.abs(rawYaw) < CURSOR_GAZE_PROFILE.yawDeadZone ? 0 : rawYaw;
    const pitch = Math.abs(rawPitch) < CURSOR_GAZE_PROFILE.pitchDeadZone ? 0 : rawPitch;
    this.attend(
      yaw * CURSOR_GAZE_PROFILE.yawDegrees,
      pitch * CURSOR_GAZE_PROFILE.pitchDegrees,
      nowMs,
      CURSOR_GAZE_PROFILE.attentionMs,
    );
  }

  releaseAttention(nowMs: number): void {
    this.targetYaw = 0;
    this.targetPitch = 0;
    this.attentionUntil = nowMs + 700;
    this.headFollowAfter = nowMs + CURSOR_GAZE_PROFILE.headFollowDelayMs;
    this.chestFollowAfter = nowMs + CURSOR_GAZE_PROFILE.chestFollowDelayMs;
  }

  setContext(context: {
    speaking: boolean;
    sleepiness: number;
    curiosity?: number;
    energy?: number;
  }): void {
    this.speaking = context.speaking;
    this.sleepiness = MathUtils.clamp(context.sleepiness, 0, 1);
    this.curiosity = MathUtils.clamp(context.curiosity ?? this.curiosity, 0, 1);
    this.energy = MathUtils.clamp(context.energy ?? this.energy, 0, 1);
  }

  reset(nowMs = 0): void {
    this.nextBlinkAt = 0;
    this.blinkStartedAt = -1;
    this.secondBlinkAt = -1;
    this.targetYaw = 0;
    this.targetPitch = 0;
    this.eyeYaw = 0;
    this.eyePitch = 0;
    this.headYaw = 0;
    this.headPitch = 0;
    this.chestYaw = 0;
    this.attentionUntil = 0;
    this.headFollowAfter = 0;
    this.chestFollowAfter = 0;
    this.speaking = false;
    this.sleepiness = 0;
    this.curiosity = 0.5;
    this.energy = 0.75;
    if (nowMs > 0) this.scheduleBlink(nowMs);
  }

  private scheduleBlink(nowMs: number): void {
    // Truncated log-normal: natural long tail, never the robotic fixed interval used by v1.
    const gaussian = normal(this.random);
    const contextFactor = (this.speaking ? 0.9 : 1) * MathUtils.lerp(1, 0.72, this.sleepiness);
    const interval = MathUtils.clamp(
      Math.exp(1.23 + gaussian * 0.42) * 1_000 * contextFactor,
      1_800,
      8_000,
    );
    this.nextBlinkAt = nowMs + interval;
  }
}

function damp(current: number, target: number, lambda: number, deltaSeconds: number): number {
  return MathUtils.lerp(current, target, 1 - Math.exp(-lambda * deltaSeconds));
}

function normal(random: () => number): number {
  const left = Math.max(random(), Number.EPSILON);
  const right = random();
  return Math.sqrt(-2 * Math.log(left)) * Math.cos(2 * Math.PI * right);
}

function truncatedNormal(
  random: () => number,
  mean: number,
  deviation: number,
  minimum: number,
  maximum: number,
): number {
  return MathUtils.clamp(mean + normal(random) * deviation, minimum, maximum);
}
