import { expect } from "@wdio/globals";
import { rmSync } from "node:fs";
import { join } from "node:path";

import { clickWhenReady, waitForDisplayed } from "../support/interactions.mjs";
import { switchToPet, switchToWorkbench } from "../support/windows.mjs";

const REMOVED_NAMES = new Set([
  "飞吻",
  "鼓掌",
  "哭泣",
  "失败",
  "对话伴随动作",
  "叉腰待机",
  "活力姿势",
  "嘻哈舞",
  "浩室舞",
  "待机",
  "拒绝",
  "快速正式鞠躬",
  "伤心待机",
  "唱歌",
  "坐姿打哈欠",
  "困倦站立待机",
  "困倦待机",
  "站立问候",
  "不耐烦站立待机",
  "说话",
  "感谢",
  "思考或确认",
  "思考",
  "疲惫待机",
  "随意手势",
  "移动恢复待机",
  "左转",
  "右转",
  "起步",
  "停步",
  "活力开心回应",
  "活力待机",
  "华丽待机",
  "力量型开心回应",
  "力量型待机",
  "害羞开心回应",
  "害羞待机",
  "标准开心回应",
  "标准待机",
  "行走",
]);

describe("Avatar Motion Runtime V5", () => {
  it("starts with appearing over waiting and returns completed actions to waiting", async () => {
    await switchToPet();
    await waitForDisplayed(".pet-avatar-hit-area", 20_000);
    await browser.waitUntil(
      async () =>
        (await $(".pet-avatar-canvas").getAttribute("data-motion-runtime")) === "v5" &&
        ((await $(".pet-avatar-canvas").getAttribute("data-motion-slots")) ?? "").includes("base"),
      { timeout: 20_000, timeoutMsg: "Waiting base motion did not start" },
    );
    expect(await $(".pet-avatar-canvas").getAttribute("data-motion-startup")).toContain(
      "openmaiwaifu.appearing",
    );

    await switchToWorkbench();
    const motionCatalog = await invokeTauri("list_motion_catalog");
    const waiting = motionCatalog.entries.find(
      (entry) => entry.id === "builtin.openmaiwaifu.waiting.3b2e83e2",
    );
    expect(waiting).toBeDefined();
    expect(
      motionCatalog.entries
        .filter((entry) => entry.source === "builtin")
        .every((entry) => entry.fallbackMotionId === waiting.id),
    ).toBe(true);
    expect(motionCatalog.entries.some((entry) => REMOVED_NAMES.has(entry.nameZh))).toBe(false);

    const userMotion = motionCatalog.entries.find(
      (entry) => entry.id === "user.desktop-e2e.ready" && entry.analysisStatus === "ready",
    );
    expect(userMotion).toBeDefined();
    await emitToPet("motion:intent-request", motionIntent("e2e:return-to-waiting", userMotion.id));
    await switchToPet();
    await browser.waitUntil(
      async () =>
        ((await $(".pet-avatar-canvas").getAttribute("data-motion-slots")) ?? "").includes(
          "action",
        ),
      { timeout: 5_000, timeoutMsg: "E2E action did not enter the action slot" },
    );
    await browser.waitUntil(
      async () => (await $(".pet-avatar-canvas").getAttribute("data-motion-slots")) === "base",
      { timeout: 15_000, timeoutMsg: "Completed action did not return to waiting base idle" },
    );

    await browser.waitUntil(
      async () => Boolean(await $(".pet-avatar-canvas").getAttribute("data-motion-ambient")),
      { timeout: 30_000, timeoutMsg: "One-shot ambient motion did not start within 25 seconds" },
    );
    const ambientMotionId = await $(".pet-avatar-canvas").getAttribute("data-motion-ambient");
    const ambientMotion = motionCatalog.entries.find((entry) => entry.id === ambientMotionId);
    expect(ambientMotion).toBeDefined();
    expect(ambientMotion.loopMode).toBe("once");
    await browser.waitUntil(
      async () =>
        ((await $(".pet-avatar-canvas").getAttribute("data-motion-slots")) ?? "").includes(
          "action",
        ),
      { timeout: 5_000, timeoutMsg: "Ambient motion did not enter the action slot" },
    );
    await browser.waitUntil(
      async () => (await $(".pet-avatar-canvas").getAttribute("data-motion-slots")) === "base",
      { timeout: 20_000, timeoutMsg: "Ambient action did not recover to waiting base idle" },
    );
  });

  it("keeps transition diagnostics and direct Pet interaction live across windows", async () => {
    await switchToWorkbench();
    await clickWhenReady('[data-testid="motion-lab-open"]');
    await waitForDisplayed('[data-testid="motion-lab-v5"]', 30_000);
    await browser.waitUntil(
      async () => {
        const text = await $(".motion-lab-diagnostics").getText();
        return text.includes("Transition") || text.includes("切换");
      },
      { timeout: 30_000, timeoutMsg: "Motion Lab transition diagnostics were not ready" },
    );
    await clickWhenReady(".motion-lab-matrix button");
    await browser.waitUntil(
      async () => {
        const rows = await $$(".motion-lab-matrix tbody tr");
        if (rows.length === 0) return false;
        const progress = await $(".motion-lab-matrix-heading p").getText();
        const [completed, total] = progress.split("/").map((value) => Number(value.trim()));
        return total > 0 && completed === total && rows.length === total;
      },
      { timeout: 120_000, timeoutMsg: "Motion Lab core transition matrix did not complete" },
    );
    const rejectedTransitions = await $$(".motion-lab-matrix tbody tr.failed");
    expect(rejectedTransitions).toHaveLength(0);

    await clickWhenReady('[data-testid="motion-lab-play-pet"]');
    await expect($('[data-testid="motion-lab-pet-status"]')).toBeDisplayed();

    await switchToPet();
    await waitForDisplayed(".pet-avatar-hit-area", 20_000);
    await browser.waitUntil(
      async () =>
        (await $(".pet-avatar-canvas").getAttribute("data-motion-runtime")) === "v5" &&
        Number(await $(".pet-avatar-canvas").getAttribute("data-motion-frame-at")) > 0,
      { timeout: 20_000, timeoutMsg: "Pet V5 frame pipeline did not start" },
    );
    await browser.waitUntil(
      async () => Boolean(await $(".pet-avatar-canvas").getAttribute("data-motion-slots")),
      { timeout: 5_000, timeoutMsg: "MotionIntent did not enter its animation graph slot" },
    );
    expect((await $(".pet-avatar-canvas").getAttribute("data-motion-slots")) ?? "").toContain(
      "base",
    );

    const hitArea = await $(".pet-avatar-hit-area");
    await clickWhenReady(".pet-avatar-hit-area");
    await browser.keys("Enter");
    await browser.waitUntil(
      async () => {
        const canvas = $(".pet-avatar-canvas");
        const interactionAt = Number(await canvas.getAttribute("data-motion-interaction-at"));
        const feedbackAt = Number(await canvas.getAttribute("data-motion-feedback-at"));
        return interactionAt > 0 && feedbackAt >= interactionAt;
      },
      { timeout: 1_000, timeoutMsg: "Direct interaction did not reach the next rendered frame" },
    );
    const directInteractionAt = Number(
      await $(".pet-avatar-canvas").getAttribute("data-motion-interaction-at"),
    );
    const directFeedbackAt = Number(
      await $(".pet-avatar-canvas").getAttribute("data-motion-feedback-at"),
    );
    expect(directFeedbackAt - directInteractionAt).toBeLessThanOrEqual(80);
    const location = await hitArea.getLocation();
    const size = await hitArea.getSize();
    await browser
      .action("pointer")
      .move({
        x: Math.round(location.x + size.width * 0.96),
        y: Math.round(location.y + size.height / 2),
      })
      .perform();
    await browser.waitUntil(
      async () =>
        Number(await $(".pet-avatar-canvas").getAttribute("data-motion-head-yaw")) > 20,
      { timeout: 2_000, timeoutMsg: "Cursor gaze did not produce the wider head turn" },
    );
    await browser
      .action("pointer")
      .move({
        x: Math.round(location.x + size.width / 2),
        y: Math.round(location.y + size.height * 0.12),
      })
      .down({ button: 0 })
      .move({
        x: Math.round(location.x + size.width / 2 + 8),
        y: Math.round(location.y + size.height * 0.12),
        duration: 180,
      })
      .move({
        x: Math.round(location.x + size.width / 2 - 8),
        y: Math.round(location.y + size.height * 0.12),
        duration: 180,
      })
      .up({ button: 0 })
      .perform();
    await browser.waitUntil(
      async () =>
        (await $(".pet-avatar-canvas").getAttribute("data-motion-interaction")) === "released",
      { timeout: 5_000, timeoutMsg: "Sustained head pat did not release naturally" },
    );

    await browser
      .action("pointer")
      .move({
        x: Math.round(location.x + size.width / 2),
        y: Math.round(location.y + size.height / 2),
      })
      .down({ button: 0 })
      .move({
        x: Math.round(location.x + size.width / 2 + 35),
        y: Math.round(location.y + size.height / 2 + 10),
        duration: 180,
      })
      .up({ button: 0 })
      .perform();
    await browser.waitUntil(
      async () =>
        ["active", "released"].includes(
          (await $(".pet-avatar-canvas").getAttribute("data-motion-drag")) ?? "",
        ),
      { timeout: 5_000, timeoutMsg: "Pet drag feedback state was not observed" },
    );

    await switchToWorkbench();
    await clickWhenReady('[data-testid="motion-lab-touch-head"]');
    await switchToPet();
    await browser.waitUntil(
      async () =>
        (await $(".pet-avatar-canvas").getAttribute("data-motion-interaction")) === "head_top",
      { timeout: 5_000, timeoutMsg: "Head interaction did not reach the Pet feedback layer" },
    );
    const canvas = $(".pet-avatar-canvas");
    expect(Number(await canvas.getAttribute("data-motion-bone-step-deg"))).toBeLessThanOrEqual(
      12.001,
    );
    expect(Number(await canvas.getAttribute("data-motion-root-step-ratio"))).toBeLessThanOrEqual(
      0.00501,
    );
    expect(Number(await canvas.getAttribute("data-motion-look-at-step-deg"))).toBeLessThanOrEqual(
      4.001,
    );
    expect(Number(await canvas.getAttribute("data-motion-foot-drift-ratio"))).toBeLessThanOrEqual(
      0.01501,
    );
    expect(
      Number.isFinite(Number(await canvas.getAttribute("data-motion-ground-penetration-ratio"))),
    ).toBe(true);

    await switchToWorkbench();
    await clickWhenReady('[data-testid="motion-lab-speech-start"]');
    await switchToPet();
    await browser.waitUntil(
      async () => (await $(".pet-avatar-canvas").getAttribute("data-motion-speech")) === "playing",
      { timeout: 5_000, timeoutMsg: "Speech did not enter the Pet speech slot" },
    );
    await switchToWorkbench();
    await clickWhenReady('[data-testid="motion-lab-speech-stop"]');
    await switchToPet();
    await browser.waitUntil(
      async () => (await $(".pet-avatar-canvas").getAttribute("data-motion-speech")) === "idle",
      { timeout: 5_000, timeoutMsg: "Speech did not release within the runtime window" },
    );

    const beforeInvalidFrame = Number(await canvas.getAttribute("data-motion-frame-at"));
    await switchToWorkbench();
    await emitToPet("motion:intent-request", {
      requestId: "e2e:invalid-motion",
      motionId: "user.missing-vrma",
      slot: "action",
      active: true,
      priority: 90,
      interruptPolicy: "immediate",
      mirror: false,
      channelWeights: [],
      locomotion: null,
    });
    await switchToPet();
    await browser.waitUntil(
      async () =>
        Number(await $(".pet-avatar-canvas").getAttribute("data-motion-frame-at")) >
        beforeInvalidFrame,
      { timeout: 2_000, timeoutMsg: "Runtime stopped after an invalid motion request" },
    );
    expect((await canvas.getAttribute("data-motion-slots")) ?? "").toContain("base");

    await switchToWorkbench();
    const motionCatalog = await invokeTauri("list_motion_catalog");
    const userMotion = motionCatalog.entries.find(
      (entry) => entry.id === "user.desktop-e2e.ready" && entry.analysisStatus === "ready",
    );
    const missingMotion = motionCatalog.entries.find(
      (entry) => entry.id === "user.desktop-e2e.missing" && entry.analysisStatus === "ready",
    );
    expect(userMotion).toBeDefined();
    expect(missingMotion).toBeDefined();
    expect(
      await invokeTauri("get_motion_runtime_asset", { request: { id: userMotion.id } }),
    ).not.toBeNull();
    await emitToPet("motion:intent-request", motionIntent("e2e:user-vrma", userMotion.id));
    await switchToPet();
    await browser.waitUntil(
      async () =>
        ((await $(".pet-avatar-canvas").getAttribute("data-motion-slots")) ?? "").includes(
          "action",
        ),
      { timeout: 5_000, timeoutMsg: "User VRMA did not enter the action slot" },
    );

    await switchToWorkbench();
    const missingBlob = join(
      process.env.HACHIMI_DATA_DIR,
      "motions-v5",
      "blobs",
      missingMotion.sha256,
      "source.vrma",
    );
    rmSync(missingBlob);
    expect(
      await invokeTauri("get_motion_runtime_asset", { request: { id: missingMotion.id } }),
    ).toBeNull();
    await emitToPet("motion:intent-request", motionIntent("e2e:missing-vrma", missingMotion.id));
    await switchToPet();
    const beforeMissingFrame = Number(
      await $(".pet-avatar-canvas").getAttribute("data-motion-frame-at"),
    );
    await browser.waitUntil(
      async () =>
        Number(await $(".pet-avatar-canvas").getAttribute("data-motion-frame-at")) >
          beforeMissingFrame &&
        Boolean(await $(".pet-avatar-canvas").getAttribute("data-motion-fallback")),
      { timeout: 5_000, timeoutMsg: "Missing user VRMA did not recover through the fallback path" },
    );
    expect((await $(".pet-avatar-canvas").getAttribute("data-motion-slots")) ?? "").toContain(
      "base",
    );

    const beforeSwitchFrame = Number(await canvas.getAttribute("data-motion-frame-at"));
    await switchToWorkbench();
    const avatarCatalog = await invokeTauri("list_avatar_models");
    const originalAvatarId = avatarCatalog.currentId;
    const replacement =
      avatarCatalog.entries.find(
        (entry) => entry.id !== originalAvatarId && entry.compatibility === "runtime_ready",
      ) ?? avatarCatalog.entries.find((entry) => entry.id === originalAvatarId);
    if (replacement) {
      await invokeTauri("select_avatar_model", { request: { id: replacement.id } });
      await emitToPet("pet:refresh-avatar", null);
      await switchToPet();
      await browser.waitUntil(
        async () =>
          Number(await $(".pet-avatar-canvas").getAttribute("data-motion-frame-at")) >
            beforeSwitchFrame &&
          ((await $(".pet-avatar-canvas").getAttribute("data-motion-slots")) ?? "").includes(
            "base",
          ),
        { timeout: 20_000, timeoutMsg: "VRM model switch did not recover to V5 base idle" },
      );
      if (originalAvatarId && replacement.id !== originalAvatarId) {
        const beforeRestoreFrame = Number(
          await $(".pet-avatar-canvas").getAttribute("data-motion-frame-at"),
        );
        await switchToWorkbench();
        await invokeTauri("select_avatar_model", { request: { id: originalAvatarId } });
        await emitToPet("pet:refresh-avatar", null);
        await switchToPet();
        await browser.waitUntil(
          async () =>
            Number(await $(".pet-avatar-canvas").getAttribute("data-motion-frame-at")) >
              beforeRestoreFrame &&
            ((await $(".pet-avatar-canvas").getAttribute("data-motion-slots")) ?? "").includes(
              "base",
            ),
          { timeout: 20_000, timeoutMsg: "Original VRM model did not recover after restoration" },
        );
      }
    }
  });
});

async function invokeTauri(command, args = {}) {
  return browser
    .executeAsync(
      (name, payload, done) => {
        window.__TAURI_INTERNALS__.invoke(name, payload).then(done, (error) => {
          const detail =
            typeof error === "string"
              ? error
              : JSON.stringify(error, Object.getOwnPropertyNames(error ?? {}));
          done({ __e2eError: detail || String(error) });
        });
      },
      command,
      args,
    )
    .then((result) => {
      if (result?.__e2eError) throw new Error(result.__e2eError);
      return result;
    });
}

async function emitToPet(event, payload) {
  const result = await invokeTauri("plugin:event|emit_to", {
    target: { kind: "AnyLabel", label: "pet" },
    event,
    payload,
  });
  return result;
}

function motionIntent(requestId, motionId) {
  return {
    requestId,
    motionId,
    slot: "action",
    active: true,
    priority: 90,
    interruptPolicy: "immediate",
    mirror: false,
    channelWeights: [],
    locomotion: null,
  };
}
