import type {
  MotionIntentRequest,
  InteractionMotionPreviewRequest,
  RuntimeControllerRequest,
  SpeechPlaybackEvent,
} from "@hachimi/contracts";
import { describe, expect, it } from "vitest";
import {
  MotionLabPetController,
  type PetMotionEmitter,
  type PetMotionEvent,
} from "./motion-lab-pet-control";

describe("MotionLabPetController", () => {
  it("plays and stops the selected catalog motion through the Pet event boundary", async () => {
    const events: Array<{
      target: string;
      event: PetMotionEvent;
      payload:
        | MotionIntentRequest
        | RuntimeControllerRequest
        | InteractionMotionPreviewRequest
        | SpeechPlaybackEvent;
    }> = [];
    const emit: PetMotionEmitter = async (target, event, payload) => {
      events.push({ target, event, payload });
    };
    const controller = new MotionLabPetController(emit);

    await controller.playMotion("builtin:wave", true);
    await controller.stopMotion();

    expect(events).toEqual([
      {
        target: "pet",
        event: "motion:intent-request",
        payload: {
          requestId: "motion-lab:pet-preview",
          motionId: "builtin:wave",
          slot: "action",
          active: true,
          priority: 70,
          interruptPolicy: "safe_point",
          mirror: true,
          channelWeights: [],
          locomotion: null,
        },
      },
      {
        target: "pet",
        event: "motion:intent-request",
        payload: {
          requestId: "motion-lab:pet-preview",
          motionId: "builtin:wave",
          slot: "action",
          active: false,
          priority: 70,
          interruptPolicy: "immediate",
          mirror: false,
          channelWeights: [],
          locomotion: null,
        },
      },
    ]);
  });

  it("clamps stage targets and emits monotonically increasing controller sequences", async () => {
    const events: Array<
      | MotionIntentRequest
      | RuntimeControllerRequest
      | InteractionMotionPreviewRequest
      | SpeechPlaybackEvent
    > = [];
    const emit: PetMotionEmitter = async (_target, _event, payload) => {
      events.push(payload);
    };
    const controller = new MotionLabPetController(emit);

    await controller.walkTo(-2);
    await controller.walkTo(2);
    await controller.stopWalking();

    expect(events).toEqual([
      {
        kind: "locomotion",
        active: true,
        target: [-0.3, null, null],
        intensity: 1,
        sequence: 1,
      },
      {
        kind: "locomotion",
        active: true,
        target: [0.3, null, null],
        intensity: 1,
        sequence: 2,
      },
      {
        kind: "locomotion",
        active: false,
        target: [0, null, null],
        intensity: 1,
        sequence: 3,
      },
    ]);
  });

  it("previews a saved interaction through the real Pet interaction path", async () => {
    const events: Array<{
      event: PetMotionEvent;
      payload:
        | MotionIntentRequest
        | RuntimeControllerRequest
        | InteractionMotionPreviewRequest
        | SpeechPlaybackEvent;
    }> = [];
    const controller = new MotionLabPetController(async (_target, event, payload) => {
      events.push({ event, payload });
    });

    await controller.previewInteraction("head_top");

    expect(events).toEqual([
      { event: "motion:preview-interaction", payload: { region: "head_top" } },
    ]);
  });

  it("drives deterministic speech start and release through the real playback event", async () => {
    const events: Array<{ event: PetMotionEvent; payload: SpeechPlaybackEvent }> = [];
    const controller = new MotionLabPetController(async (_target, event, payload) => {
      if (event === "voice:playback") {
        events.push({ event, payload: payload as SpeechPlaybackEvent });
      }
    });

    await controller.startSpeech();
    await controller.stopSpeech();

    expect(events.map(({ payload }) => payload.phase)).toEqual(["prepared", "playing", "stopped"]);
    expect(events.map(({ payload }) => payload.sequence)).toEqual([1, 2, 3]);
    expect(events[0]?.payload.timeline?.jawOpen).toHaveLength(8);
  });
});
