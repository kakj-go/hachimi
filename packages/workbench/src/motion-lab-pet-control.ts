import { emitTo } from "@tauri-apps/api/event";
import type {
  ClipMotionRequest,
  InteractionMotionPreviewRequest,
  InteractionRegion,
  RuntimeControllerRequest,
} from "@hachimi/contracts";

export type PetMotionEvent =
  | "motion:clip-request"
  | "motion:controller-request"
  | "motion:preview-interaction";
export type PetMotionEmitter = (
  target: string,
  event: PetMotionEvent,
  payload: ClipMotionRequest | RuntimeControllerRequest | InteractionMotionPreviewRequest,
) => Promise<void>;

const REQUEST_ID = "motion-lab:pet-preview";

/** Sends Motion Lab QA requests through the same event boundary consumed by the real Pet. */
export class MotionLabPetController {
  private sequence = 0;
  private currentMotionId = "";

  constructor(private readonly emitter: PetMotionEmitter = emitTo) {}

  async playMotion(motionId: string, mirror = false): Promise<void> {
    if (!motionId.trim()) throw new Error("A motion must be selected before Pet playback.");
    this.currentMotionId = motionId;
    await this.emitter("pet", "motion:clip-request", {
      requestId: REQUEST_ID,
      motionId,
      active: true,
      priority: 70,
      mirror,
      channelWeights: [],
    });
  }

  async stopMotion(): Promise<void> {
    await this.emitter("pet", "motion:clip-request", {
      requestId: REQUEST_ID,
      motionId: this.currentMotionId,
      active: false,
      priority: 70,
      mirror: false,
      channelWeights: [],
    });
    this.currentMotionId = "";
  }

  async walkTo(targetX: number): Promise<void> {
    await this.sendLocomotion(true, Math.max(-0.3, Math.min(0.3, targetX)));
  }

  async stopWalking(): Promise<void> {
    await this.sendLocomotion(false, 0);
  }

  async previewInteraction(region: InteractionRegion): Promise<void> {
    await this.emitter("pet", "motion:preview-interaction", { region });
  }

  private async sendLocomotion(active: boolean, targetX: number): Promise<void> {
    this.sequence += 1;
    await this.emitter("pet", "motion:controller-request", {
      kind: "locomotion",
      active,
      target: [targetX, null, null],
      intensity: 1,
      sequence: this.sequence,
    });
  }
}
