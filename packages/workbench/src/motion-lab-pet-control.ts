import { emitTo } from "@tauri-apps/api/event";
import type {
  MotionIntentRequest,
  MotionSlot,
  InteractionMotionPreviewRequest,
  InteractionRegion,
  RuntimeControllerRequest,
  SpeechPlaybackEvent,
} from "@hachimi/contracts";

export type PetMotionEvent =
  | "motion:intent-request"
  | "motion:controller-request"
  | "motion:preview-interaction"
  | "voice:playback";
export type PetMotionEmitter = (
  target: string,
  event: PetMotionEvent,
  payload:
    | MotionIntentRequest
    | RuntimeControllerRequest
    | InteractionMotionPreviewRequest
    | SpeechPlaybackEvent,
) => Promise<void>;

const REQUEST_ID = "motion-lab:pet-preview";

/** Sends Motion Lab QA requests through the same event boundary consumed by the real Pet. */
export class MotionLabPetController {
  private sequence = 0;
  private speechSequence = 0;
  private currentMotionId = "";
  private currentSlot: MotionSlot = "action";

  constructor(private readonly emitter: PetMotionEmitter = emitTo) {}

  async playMotion(motionId: string, mirror = false, slot: MotionSlot = "action"): Promise<void> {
    if (!motionId.trim()) throw new Error("A motion must be selected before Pet playback.");
    this.currentMotionId = motionId;
    this.currentSlot = slot;
    await this.emitter("pet", "motion:intent-request", {
      requestId: REQUEST_ID,
      motionId,
      slot,
      active: true,
      priority: 70,
      interruptPolicy: "safe_point",
      mirror,
      channelWeights: [],
      locomotion: null,
    });
  }

  async stopMotion(): Promise<void> {
    await this.emitter("pet", "motion:intent-request", {
      requestId: REQUEST_ID,
      motionId: this.currentMotionId,
      slot: this.currentSlot,
      active: false,
      priority: 70,
      interruptPolicy: "immediate",
      mirror: false,
      channelWeights: [],
      locomotion: null,
    });
    this.currentMotionId = "";
    this.currentSlot = "action";
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

  async startSpeech(): Promise<void> {
    const prepared = this.speechEvent("prepared", 0, {
      frameDurationMs: 20,
      jawOpen: [0, 96, 180, 112, 32, 156, 64, 0],
      visemes: null,
      quality: "energy_locked",
    });
    await this.emitter("pet", "voice:playback", prepared);
    await this.emitter("pet", "voice:playback", this.speechEvent("playing", 0, null));
  }

  async stopSpeech(): Promise<void> {
    await this.emitter("pet", "voice:playback", this.speechEvent("stopped", 160, null));
  }

  private speechEvent(
    phase: SpeechPlaybackEvent["phase"],
    mediaPositionMs: number,
    timeline: SpeechPlaybackEvent["timeline"],
  ): SpeechPlaybackEvent {
    this.speechSequence += 1;
    return {
      playbackId: "motion-lab:voice-preview",
      runId: null,
      source: "pet_turn",
      phase,
      mediaPositionMs,
      durationMs: 1_000,
      sequence: this.speechSequence,
      timeline,
      segmentIndex: 0,
      textStart: 0,
      textEnd: 0,
      displayText: null,
    };
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
