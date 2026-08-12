import { MathUtils } from "three";

export type StageLocomotionPhase = "idle" | "start" | "walk" | "stop" | "turn";

export interface StageLocomotionFrame {
  phase: StageLocomotionPhase;
  positionX: number;
  facing: -1 | 1;
  speed: number;
  distanceRemaining: number;
}

/** Small bounded 1D stage navigator; positions and speeds are normalized by avatar height. */
export class StageLocomotionController {
  private positionX = 0;
  private targetX = 0;
  private speed = 0;
  private facing: -1 | 1 = 1;
  private phase: StageLocomotionPhase = "idle";

  reset(positionX = 0): void {
    this.positionX = clampStage(positionX);
    this.targetX = this.positionX;
    this.speed = 0;
    this.facing = 1;
    this.phase = "idle";
  }

  walkTo(targetX: number): void {
    this.targetX = clampStage(targetX);
    const distance = this.targetX - this.positionX;
    if (Math.abs(distance) < 0.012) return;
    const nextFacing: -1 | 1 = distance < 0 ? -1 : 1;
    this.phase = nextFacing === this.facing ? "start" : "turn";
    this.facing = nextFacing;
  }

  stop(): void {
    this.targetX = this.positionX;
    if (this.phase !== "idle") this.phase = "stop";
  }

  update(deltaSeconds: number): StageLocomotionFrame {
    const delta = MathUtils.clamp(deltaSeconds, 0, 0.05);
    const distance = this.targetX - this.positionX;
    const direction = Math.sign(distance);
    const brakingSpeed = Math.sqrt(Math.max(2 * 1.2 * Math.abs(distance), 0));
    const desired = direction * Math.min(0.28, brakingSpeed);
    const acceleration = this.phase === "stop" ? 1.8 : 1.2;
    this.speed = moveTowards(this.speed, desired, acceleration * delta);
    if (Math.abs(distance) < 0.006 && Math.abs(this.speed) < 0.025) {
      this.positionX = this.targetX;
      this.speed = 0;
      this.phase = "idle";
    } else {
      this.positionX = clampStage(this.positionX + this.speed * delta);
      if (this.phase === "start") {
        if (Math.abs(this.speed) > 0.08) this.phase = "walk";
      } else if (this.phase === "turn") {
        if (Math.sign(this.speed) === this.facing && Math.abs(this.speed) > 0.08) {
          this.phase = "walk";
        }
      } else if (Math.abs(distance) < 0.045) {
        this.phase = "stop";
      }
    }
    return {
      phase: this.phase,
      positionX: this.positionX,
      facing: this.facing,
      speed: Math.abs(this.speed),
      distanceRemaining: Math.abs(this.targetX - this.positionX),
    };
  }
}

function clampStage(value: number): number {
  return MathUtils.clamp(Number.isFinite(value) ? value : 0, -0.3, 0.3);
}

function moveTowards(value: number, target: number, maximumDelta: number): number {
  if (Math.abs(target - value) <= maximumDelta) return target;
  return value + Math.sign(target - value) * maximumDelta;
}
