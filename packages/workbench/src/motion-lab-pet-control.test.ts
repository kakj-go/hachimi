import type {
  ClipMotionRequest,
  InteractionMotionPreviewRequest,
  RuntimeControllerRequest,
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
      payload: ClipMotionRequest | RuntimeControllerRequest | InteractionMotionPreviewRequest;
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
        event: "motion:clip-request",
        payload: {
          requestId: "motion-lab:pet-preview",
          motionId: "builtin:wave",
          active: true,
          priority: 70,
          mirror: true,
          channelWeights: [],
        },
      },
      {
        target: "pet",
        event: "motion:clip-request",
        payload: {
          requestId: "motion-lab:pet-preview",
          motionId: "builtin:wave",
          active: false,
          priority: 70,
          mirror: false,
          channelWeights: [],
        },
      },
    ]);
  });

  it("clamps stage targets and emits monotonically increasing controller sequences", async () => {
    const events: Array<
      ClipMotionRequest | RuntimeControllerRequest | InteractionMotionPreviewRequest
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
      payload: ClipMotionRequest | RuntimeControllerRequest | InteractionMotionPreviewRequest;
    }> = [];
    const controller = new MotionLabPetController(async (_target, event, payload) => {
      events.push({ event, payload });
    });

    await controller.previewInteraction("head_top");

    expect(events).toEqual([
      { event: "motion:preview-interaction", payload: { region: "head_top" } },
    ]);
  });
});
