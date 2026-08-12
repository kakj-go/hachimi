import type { InteractionRegion } from "@hachimi/contracts";
import { MathUtils, Vector2 } from "three";

export interface InteractionFeedbackFrame {
  active: boolean;
  region: InteractionRegion;
  direction: -1 | 1;
  pressure: number;
  dragVelocity: Vector2;
  release: number;
}

/** Continuous local feedback state. Pointer updates mutate this state instead of replaying clips. */
export class InteractionFeedbackRuntime {
  private region: InteractionRegion = "generic";
  private direction: -1 | 1 = 1;
  private pressure = 0;
  private dragVelocity = new Vector2();
  private active = false;
  private releasedAt = Number.NEGATIVE_INFINITY;

  begin(region: InteractionRegion, direction: -1 | 1, pressure: number, nowMs: number): void {
    this.region = region;
    this.direction = direction;
    this.pressure = MathUtils.clamp(pressure, 0, 1);
    this.active = true;
    this.releasedAt = nowMs;
  }

  update(pressure: number, direction = this.direction): void {
    this.pressure = MathUtils.clamp(pressure, 0, 1);
    this.direction = direction;
  }

  setDrag(active: boolean, velocity: Vector2, nowMs: number): void {
    this.dragVelocity.copy(velocity);
    if (active) this.begin("generic", velocity.x < 0 ? -1 : 1, velocity.length(), nowMs);
    else this.end(nowMs);
  }

  end(nowMs: number): void {
    this.active = false;
    this.releasedAt = nowMs;
  }

  frame(nowMs: number): InteractionFeedbackFrame {
    const release = this.active ? 1 : Math.max(1 - (nowMs - this.releasedAt) / 220, 0);
    return {
      active: this.active,
      region: this.region,
      direction: this.direction,
      pressure: this.active ? this.pressure : this.pressure * release,
      dragVelocity: this.dragVelocity.clone().multiplyScalar(release),
      release,
    };
  }

  reset(): void {
    this.active = false;
    this.pressure = 0;
    this.dragVelocity.set(0, 0);
    this.releasedAt = Number.NEGATIVE_INFINITY;
  }
}
