import { describe, expect, it } from "vitest";
import { FaceExpressionMixer, FaceGazeRuntime } from "./face-gaze-runtime";

describe("FaceExpressionMixer", () => {
  it("composes four layers while letting higher layers replace the same expression", () => {
    const mixer = new FaceExpressionMixer();
    mixer.beginFrame();
    mixer.set("base", "happy", 0.1);
    mixer.set("behavior", "happy", 0.35);
    mixer.set("reaction", "surprised", 0.8);
    mixer.set("blink_viseme", "blink", 0.9);
    mixer.set("blink_viseme", "aa", 0.8);
    mixer.set("blink_viseme", "ih", 0.6);

    const values = mixer.resolve();
    expect(values.get("happy")).toBeCloseTo(0.35 / 1.15, 5);
    expect(values.get("surprised")).toBeCloseTo(0.8 / 1.15, 5);
    expect(values.get("blink")).toBe(0.9);
    expect((values.get("aa") ?? 0) + (values.get("ih") ?? 0)).toBeCloseTo(1, 6);
  });
});

describe("FaceGazeRuntime", () => {
  it("keeps the eyes bounded while recruiting a faster, wider head and chest turn", () => {
    const gaze = new FaceGazeRuntime(() => 0.5);
    gaze.reset(1_000);
    gaze.attend(40, 30, 1_000, 1_000);
    const first = gaze.update(1_016, 0.016);
    const settled = gaze.update(1_500, 0.484);

    expect(Math.abs(first.eyeYaw)).toBeGreaterThan(Math.abs(first.headYaw));
    expect(first.headYaw).toBe(0);
    expect(first.chestYaw).toBe(0);
    expect(settled.eyeYaw).toBeLessThanOrEqual(28);
    expect(settled.eyePitch).toBeLessThanOrEqual(18);
    expect(settled.headYaw).toBeGreaterThan(23);
    expect(settled.headPitch).toBeGreaterThan(12);
    expect(settled.chestYaw).toBeGreaterThan(7);
  });

  it("does not recruit the chest for small cursor movements", () => {
    const gaze = new FaceGazeRuntime(() => 0.5);
    gaze.reset(1_000);
    gaze.attend(13, 6, 1_000, 1_000);

    const settled = gaze.update(1_500, 0.5);

    expect(settled.headYaw).toBeGreaterThan(9);
    expect(settled.chestYaw).toBe(0);
  });

  it("maps cursor input through the dead zone and wider gaze profile", () => {
    const gaze = new FaceGazeRuntime(() => 0.5);
    gaze.reset(1_000);
    gaze.attendCursor(0.03, 0.03, 1_000);
    const centered = gaze.update(1_500, 0.5);
    gaze.attendCursor(1, -1, 1_500);
    const corner = gaze.update(2_000, 0.5);

    expect(centered.headYaw).toBe(0);
    expect(centered.headPitch).toBe(0);
    expect(corner.headYaw).toBeGreaterThan(23);
    expect(corner.headPitch).toBeLessThan(-11);
  });

  it("uses curiosity and energy to vary unconstrained micro-saccades", () => {
    const quiet = new FaceGazeRuntime(() => 0.2);
    quiet.reset(1_000);
    quiet.setContext({ speaking: false, sleepiness: 0, curiosity: 0, energy: 1 });
    const quietFrame = quiet.update(1_016, 1);

    const curious = new FaceGazeRuntime(() => 0.2);
    curious.reset(1_000);
    curious.setContext({ speaking: false, sleepiness: 0, curiosity: 1, energy: 1 });
    const curiousFrame = curious.update(1_016, 1);

    expect(quietFrame.eyeYaw).toBe(0);
    expect(Math.abs(curiousFrame.eyeYaw)).toBeGreaterThan(0);
  });
});
