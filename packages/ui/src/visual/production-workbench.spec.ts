import { expect, test, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { resolve } from "node:path";
import { DEFAULT_APPEARANCE } from "../theme/context";

const appearance = structuredClone(DEFAULT_APPEARANCE);
const motionEntry = {
  id: "builtin.openmaiwaifu.standard-waiting.7c7cd057",
  source: "builtin",
  protected: true,
  name: "standard waiting",
  nameZh: "标准待机",
  description: "OpenMaiWaifu idle",
  descriptionZh: "OpenMaiWaifu 标准待机",
  fileName: "7c7cd0574c34b20a1e9cccbae3abcdc7999cd406edb7f0323e5e2aa0be4e02d5.vrma",
  sha256: "7c7cd0574c34b20a1e9cccbae3abcdc7999cd406edb7f0323e5e2aa0be4e02d5",
  sizeBytes: 200000,
  durationMs: 11667,
  category: "idle",
  tags: ["idle", "waiting"],
  playbackMode: "loop",
  rootMode: "in_place",
  channels: ["full_body", "fingers"],
  animatedBones: ["hips", "leftIndexProximal", "rightIndexProximal"],
  fingerBoneCount: 30,
  hasFingerMotion: true,
  hasExpression: false,
  hasLookAt: false,
  mirrorable: true,
  transitionInMs: 320,
  transitionOutMs: 350,
  sourceProject: "OpenMaiWaifu",
  sourcePaths: ["standard-waiting.vrma"],
  warnings: [],
};

const initialSettings = {
  schemaVersion: 8,
  developerMode: false,
  theme: "dark",
  locale: "zh-CN",
  alwaysOnTop: true,
  petPlacement: null,
  llm: {
    baseUrl: "http://localhost:11434/v1",
    modelName: "gemma4:e4b",
    maxInputTokens: 0,
    maxOutputTokens: 0,
  },
  voice: { muted: false, speedPercent: 100, computeMode: "auto" },
  appearance,
};

async function installTauriMocks(page: Page) {
  await page.addInitScript(
    ({ appearance, initialSettings, motionEntry }) => {
      let settings = initialSettings;
      const runtimeRequirements = [
        "vrm_format",
        "skinned_mesh",
        "complete_humanoid",
        "standard_blinks",
        "five_visemes",
        "standard_emotions",
        "look_at",
        "mtoon",
        "spring_bone",
        "spring_collider",
        "skin_weights",
        "resource_budget",
      ].map((requirement) => ({ requirement, passed: true, detail: "" }));
      const runtimeStatistics = {
        nodeCount: 86,
        meshCount: 3,
        primitiveCount: 4,
        triangleCount: 42_800,
        materialCount: 9,
        textureCount: 12,
        boneCount: 67,
        animationCount: 0,
        morphTargetCount: 34,
        maxTextureDimension: 2048,
        estimatedTextureMemoryBytes: 100_663_296,
      };
      const runtimeAssessment = {
        compatibility: "runtime_ready",
        detectorVersion: 4,
        capabilities: [
          "renderable_mesh",
          "skinned_mesh",
          "humanoid_skeleton",
          "blink",
          "viseme",
          "look_at",
          "spring_bone",
          "m_toon",
          "spring_bone_collider",
          "five_finger_hands",
          "five_visemes",
          "standard_expressions",
          "standard_motion_retarget",
          "runtime_ready",
        ],
        statistics: runtimeStatistics,
        requirements: runtimeRequirements,
        issues: [],
      };
      let voiceRuntime = {
        available: true,
        muted: false,
        modelId: "builtin-melo-zh-en",
        voiceName: "Hachimi 中文女声",
        speaking: false,
        speedPercent: 100,
        provider: "sherpa_onnx_vits",
        computeMode: "auto",
        backend: "cpu",
        fallbackReason: null,
        loading: false,
        languages: ["zh-CN"],
        speakerCount: 1,
        speakerId: 0,
      };
      let speechRecognition = {
        installed: true,
        installing: false,
        bundled: true,
        modelName: "SenseVoice-Small INT8",
        provider: "sherpa-onnx 1.13.4",
        languages: ["zh-CN", "en-US", "ja-JP", "ko-KR", "yue"],
        sizeBytes: 237_431_441,
        computeMode: "auto",
        backend: "direct_ml",
        fallbackReason: null,
        loading: false,
        error: null,
      };
      let voices = {
        entries: [
          {
            id: "builtin-melo-zh-en",
            name: "Hachimi 中英双语女声（MeloTTS）",
            sha256: "e58351ed7149f290a54534538badd4077cdbe6fddc964b24d0bee870415d1514",
            originalFileName: "vits-melo-tts-zh_en.tar.bz2",
            sizeBytes: 167_006_755,
            origin: "built_in",
            modelType: "melo-vits",
            languages: ["zh-CN", "en-US"],
            sampleRate: 44_100,
            speakerCount: 1,
            speakerId: 0,
            licenseSummary: "MIT",
            licenseWarning: false,
            protected: true,
            importedAt: "0",
          },
        ],
        currentId: "builtin-melo-zh-en",
      };
      let avatars = {
        entries: [
          {
            id: "mimi",
            name: "Mimi",
            originalFileName: "mimi.vrm",
            sizeBytes: 4096,
            sha256: "1234567890abcdef1234567890abcdef",
            importedAt: "1767225600000",
            isCurrent: true,
            format: "vrm0",
            assessment: runtimeAssessment,
          },
        ],
        currentId: "mimi" as string | null,
      };
      const gestureEntry = {
        ...motionEntry,
        id: "builtin.clawatar.head-nod-yes.mock",
        name: "Head Nod Yes",
        nameZh: "点头同意",
        description: "A natural affirmative head nod.",
        descriptionZh: "自然的肯定点头动作。",
        category: "gesture",
        playbackMode: "once",
        durationMs: 1800,
        fingerBoneCount: 0,
        hasFingerMotion: false,
      };
      let motions = {
        entries: [motionEntry, gestureEntry],
        bindings: [] as Array<{
          region: string;
          motionId: string;
          cooldownMs: number;
          mirrorBySide: boolean;
        }>,
        disabledMotionIds: [] as string[],
      };
      const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
      const testState = { failNextUpdate: false };
      let nextCallbackId = 1;
      const callbacks = new Map<number, (data: unknown) => unknown>();
      const internals = {
        transformCallback(callback: ((data: unknown) => unknown) | undefined, once = false) {
          const id = nextCallbackId++;
          callbacks.set(id, (data) => {
            if (once) callbacks.delete(id);
            return callback?.(data);
          });
          return id;
        },
        unregisterCallback(id: number) {
          callbacks.delete(id);
        },
        runCallback(id: number, data: unknown) {
          callbacks.get(id)?.(data);
        },
        callbacks,
        async invoke(command: string, args: Record<string, unknown> = {}) {
          calls.push({ command, args });
          if (command === "get_bootstrap_state") {
            return {
              protocolVersion: 17,
              windowKind: "workbench",
              locale: "zh-CN",
              theme: "dark",
              appearance,
              alwaysOnTop: true,
              featureFlags: {
                workbench: true,
                motionLab: true,
                workspaceTools: false,
                browserControl: false,
                computerObserve: false,
                computerControl: false,
                remoteTts: false,
                remoteGateway: false,
                connectorPlugins: false,
              },
            };
          }
          if (command === "get_settings") return settings;
          if (command === "reset_local_data") return null;
          if (command === "update_settings") {
            if (testState.failNextUpdate) {
              testState.failNextUpdate = false;
              throw new Error("mock settings write failed");
            }
            settings = args.settings as typeof initialSettings;
            return settings;
          }
          if (command === "import_theme_profile") {
            const scheme = args.scheme as "light" | "dark";
            const id = `imported-${scheme}`;
            const source = settings.appearance.themes.find((profile) => profile.scheme === scheme)!;
            const imported = {
              ...source,
              id,
              name: `Imported ${scheme}`,
              builtin: false,
              accent: scheme === "light" ? "#7C3AED" : "#22C55E",
            };
            settings = {
              ...settings,
              appearance: {
                ...settings.appearance,
                ...(scheme === "light" ? { lightThemeId: id } : { darkThemeId: id }),
                themes: [
                  ...settings.appearance.themes.filter((profile) => profile.id !== id),
                  imported,
                ],
              },
            };
            return settings;
          }
          if (command === "copy_theme_profile") return null;
          if (command === "reset_theme_profile") {
            const profileId = args.profileId as string;
            settings = {
              ...settings,
              appearance: {
                ...settings.appearance,
                themes: settings.appearance.themes.map((profile) => {
                  if (profile.id !== profileId) return profile;
                  return appearance.themes.find((item) => item.id === profileId) ?? profile;
                }),
              },
            };
            return settings;
          }
          if (command === "delete_theme_profile") {
            const profileId = args.profileId as string;
            const profile = settings.appearance.themes.find((item) => item.id === profileId);
            if (!profile || profile.builtin) throw new Error("Built-in themes cannot be deleted");
            settings = {
              ...settings,
              appearance: {
                ...settings.appearance,
                lightThemeId:
                  settings.appearance.lightThemeId === profileId
                    ? "codex-light"
                    : settings.appearance.lightThemeId,
                darkThemeId:
                  settings.appearance.darkThemeId === profileId
                    ? "codex-dark"
                    : settings.appearance.darkThemeId,
                themes: settings.appearance.themes.filter((item) => item.id !== profileId),
              },
            };
            return settings;
          }
          if (command === "set_always_on_top") {
            settings = { ...settings, alwaysOnTop: Boolean(args.enabled) };
            return settings;
          }
          if (command === "get_llm_settings") {
            return { ...settings.llm, apiKeyConfigured: false };
          }
          if (command === "save_llm_settings") {
            const input = args.input as typeof settings.llm;
            settings = { ...settings, llm: { ...settings.llm, ...input } };
            return {
              ...settings.llm,
              apiKeyConfigured: Boolean((input as { apiKey?: string }).apiKey),
            };
          }
          if (command === "save_and_test_llm_settings") {
            return { success: true, latencyMs: 12, responsePreview: "mock response" };
          }
          if (command === "list_avatar_models") return avatars;
          if (command === "list_motion_catalog") return motions;
          if (command === "get_motion_runtime_asset") {
            const requestedId = (args.request as { id: string }).id;
            return {
              entry: motions.entries.find((entry) => entry.id === requestedId) ?? motionEntry,
              assetUrl: `http://hachimi-motion.localhost/builtin/${motionEntry.fileName}`,
            };
          }
          if (command === "inspect_motion_file") {
            return {
              token: "motion-token",
              originalFileName: "custom-wave.vrma",
              sizeBytes: 4096,
              sha256: "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd",
              durationMs: 2400,
              animatedBones: ["hips", "leftHand", "leftIndexProximal"],
              fingerBoneCount: 30,
              hasExpression: false,
              hasLookAt: false,
              warnings: [],
            };
          }
          if (command === "commit_motion_import") {
            const request = args.request as {
              name: string;
              description: string;
              category: string;
              playbackMode: string;
              rootMode: string;
              interactionRegion: string | null;
            };
            motions = {
              ...motions,
              entries: [
                ...motions.entries,
                {
                  ...motionEntry,
                  id: "user.custom-wave",
                  source: "user",
                  protected: false,
                  name: request.name,
                  nameZh: request.name,
                  description: request.description,
                  descriptionZh: request.description,
                  category: request.category,
                  playbackMode: request.playbackMode,
                  rootMode: request.rootMode,
                  sourceProject: "User",
                },
              ],
            };
            if (request.interactionRegion) {
              const imported = motions.entries.at(-1)!;
              motions = {
                ...motions,
                bindings: [
                  ...motions.bindings.filter(
                    (binding) => binding.region !== request.interactionRegion,
                  ),
                  {
                    region: request.interactionRegion,
                    motionId: imported.id,
                    cooldownMs: 2200,
                    mirrorBySide: false,
                  },
                ],
              };
            }
            return motions;
          }
          if (command === "delete_user_motion") {
            const id = (args.request as { id: string }).id;
            motions = {
              ...motions,
              entries: motions.entries.filter((entry) => entry.id !== id),
              bindings: motions.bindings.filter((binding) => binding.motionId !== id),
            };
            return motions;
          }
          if (command === "update_motion_metadata") return motions;
          if (command === "set_interaction_motion_binding") {
            const request = args.request as {
              region: string;
              motionId: string | null;
              cooldownMs?: number | null;
              mirrorBySide?: boolean | null;
            };
            const current = motions.bindings.find((binding) => binding.region === request.region);
            motions = {
              ...motions,
              bindings: request.motionId
                ? [
                    ...motions.bindings.filter((binding) => binding.region !== request.region),
                    {
                      region: request.region,
                      motionId: request.motionId,
                      cooldownMs: request.cooldownMs ?? current?.cooldownMs ?? 2200,
                      mirrorBySide:
                        request.mirrorBySide ??
                        current?.mirrorBySide ??
                        request.region.includes("left"),
                    },
                  ]
                : motions.bindings.filter((binding) => binding.region !== request.region),
            };
            return motions;
          }
          if (command === "clear_motion_interaction_bindings") {
            const motionId = (args.request as { motionId: string }).motionId;
            motions = {
              ...motions,
              bindings: motions.bindings.filter((binding) => binding.motionId !== motionId),
            };
            return motions;
          }
          if (command === "set_motion_enabled") {
            const request = args.request as { id: string; enabled: boolean };
            motions = {
              ...motions,
              disabledMotionIds: request.enabled
                ? motions.disabledMotionIds.filter((id) => id !== request.id)
                : [...new Set([...motions.disabledMotionIds, request.id])],
            };
            return motions;
          }
          if (command === "reset_motion_bindings") {
            motions = { ...motions, bindings: [] };
            return motions;
          }
          if (command === "reset_motion_binding") {
            const region = (args.request as { region: string }).region;
            motions = {
              ...motions,
              bindings: motions.bindings.filter((binding) => binding.region !== region),
            };
            return motions;
          }
          if (command === "get_current_avatar_asset") {
            return {
              entryId: "mimi",
              name: "Mimi",
              sha256: "1234567890abcdef1234567890abcdef",
              assetUrl: "http://hachimi-avatar.localhost/mimi",
              format: "vrm0",
              profile: {},
            };
          }
          if (command === "get_avatar_runtime_asset") {
            return {
              entryId: "mimi",
              name: "Mimi",
              sha256: "1234567890abcdef1234567890abcdef",
              assetUrl: "http://hachimi-avatar.localhost/mimi",
              format: "vrm0",
              profile: {},
            };
          }
          if (command === "inspect_avatar_model") {
            return {
              token: "inspection-token",
              originalFileName: "luna.vrm",
              sizeBytes: 8192,
              sha256: "abcdef1234567890abcdef1234567890",
              format: "vrm1",
              assessment: runtimeAssessment,
            };
          }
          if (command === "commit_avatar_model_import") {
            const name = (args.request as { name: string }).name;
            avatars = {
              entries: [
                ...avatars.entries.map((entry) => ({ ...entry, isCurrent: false })),
                {
                  id: "luna",
                  name,
                  originalFileName: "luna.vrm",
                  sizeBytes: 8192,
                  sha256: "abcdef1234567890abcdef1234567890",
                  importedAt: "1767225600100",
                  isCurrent: true,
                  format: "vrm1",
                  assessment: runtimeAssessment,
                },
              ],
              currentId: "luna",
            };
            return avatars;
          }
          if (command === "cancel_avatar_model_import") return null;
          if (command === "select_avatar_model") {
            const id = (args.request as { id: string }).id;
            avatars = {
              entries: avatars.entries.map((entry) => ({ ...entry, isCurrent: entry.id === id })),
              currentId: id,
            };
            return avatars;
          }
          if (command === "delete_avatar_model") {
            const id = (args.request as { id: string }).id;
            avatars = {
              entries: avatars.entries.filter((entry) => entry.id !== id),
              currentId: null,
            };
            return avatars;
          }
          if (command === "get_voice_runtime_state") return voiceRuntime;
          if (command === "get_speech_recognition_state") return speechRecognition;
          if (command === "update_speech_recognition_settings") {
            speechRecognition = {
              ...speechRecognition,
              computeMode: (args.input as { computeMode: string }).computeMode,
              backend:
                (args.input as { computeMode: string }).computeMode === "cpu" ? "cpu" : "direct_ml",
            };
            return speechRecognition;
          }
          if (command === "list_voice_models") return voices;
          if (command === "inspect_voice_model") {
            return {
              token: "voice-inspection-token",
              originalFileName: "vits-melo-tts-zh_en.tar.bz2",
              sizeBytes: 32_000_000,
              sha256: "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd",
              modelType: "vits-melo",
              languages: ["zh-CN", "en-US"],
              sampleRate: 44_100,
              speakerCount: 1,
              suggestedSpeakerId: 0,
              requiredFiles: [
                "vits-melo-tts-zh_en/model.onnx",
                "vits-melo-tts-zh_en/tokens.txt",
                "vits-melo-tts-zh_en/lexicon.txt",
              ],
              licenseSummary: "License: CC-BY-4.0",
              licenseWarning: false,
              compatible: true,
              issues: [],
            };
          }
          if (command === "commit_voice_model_import") {
            const request = args.request as {
              name: string;
              licenseAcknowledged: boolean;
              speakerId: number;
            };
            voices = {
              ...voices,
              entries: [
                ...voices.entries,
                {
                  id: "melo",
                  name: request.name,
                  sha256: "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd",
                  originalFileName: "vits-melo-tts-zh_en.tar.bz2",
                  sizeBytes: 32_000_000,
                  origin: "imported",
                  modelType: "vits-melo",
                  languages: ["zh-CN", "en-US"],
                  sampleRate: 44_100,
                  speakerCount: 1,
                  speakerId: request.speakerId,
                  licenseSummary: "License: CC-BY-4.0",
                  licenseWarning: false,
                  protected: false,
                  importedAt: "1767225600000",
                },
              ],
            };
            return voices;
          }
          if (command === "cancel_voice_model_import") return null;
          if (command === "select_voice_model") {
            voices = {
              ...voices,
              currentId: (args.request as { id: string }).id,
            };
            return voices;
          }
          if (command === "delete_voice_model") {
            const id = (args.request as { id: string }).id;
            voices = {
              entries: voices.entries.filter((entry) => entry.id !== id),
              currentId: voices.currentId === id ? "builtin-melo-zh-en" : voices.currentId,
            };
            return voices;
          }
          if (command === "update_voice_settings") {
            voiceRuntime = {
              ...voiceRuntime,
              ...(args.input as { speedPercent: number; computeMode: string }),
            };
            return voiceRuntime;
          }
          if (command === "set_muted") {
            voiceRuntime = { ...voiceRuntime, muted: Boolean(args.muted) };
            return voiceRuntime;
          }
          if (command === "preview_default_voice") {
            voiceRuntime = { ...voiceRuntime, speaking: true };
            return voiceRuntime;
          }
          if (command === "stop_speech") {
            voiceRuntime = { ...voiceRuntime, speaking: false };
            return voiceRuntime;
          }
          if (command === "plugin:event|listen") return args.handler;
          return null;
        },
        convertFileSrc(path: string) {
          return path;
        },
        metadata: {
          currentWindow: { label: "workbench" },
          currentWebview: { label: "workbench" },
        },
      };
      Object.assign(window, {
        __TAURI_INTERNALS__: internals,
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => undefined },
        __HACHIMI_TEST_CALLS__: calls,
        __HACHIMI_TEST_STATE__: testState,
      });
    },
    { appearance, initialSettings, motionEntry },
  );
}

async function installMotionLabAssets(page: Page) {
  await page.route("http://hachimi-avatar.localhost/mimi", (route) =>
    route.fulfill({
      path: resolve(
        import.meta.dirname,
        "../../../../assets/avatar-default/2639776812528692620/2639776812528692620.vrm",
      ),
      contentType: "model/gltf-binary",
    }),
  );
  await page.route("http://hachimi-motion.localhost/builtin/*.vrma", (route) =>
    route.fulfill({
      path: resolve(
        import.meta.dirname,
        `../../../../assets/avatar-motions-v4/builtin/${motionEntry.fileName}`,
      ),
      contentType: "model/gltf-binary",
    }),
  );
}

for (const viewport of [
  { name: "1855x1343", width: 1855, height: 1343 },
  { name: "1280x800", width: 1280, height: 800 },
  { name: "960x640", width: 960, height: 640 },
] as const) {
  test(`production workbench ${viewport.name}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await installTauriMocks(page);
    await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
    await expect(page.getByRole("heading", { name: /hachimi-code/ })).toBeVisible();
    await expect(page.locator("html")).toHaveJSProperty("scrollWidth", viewport.width);
    await expect(page).toHaveScreenshot(`production-home-${viewport.name}.png`, {
      animations: "disabled",
    });
  });
}

for (const viewport of [
  { name: "1855x1343", width: 1855, height: 1343 },
  { name: "960x640", width: 960, height: 640 },
] as const) {
  test(`production appearance ${viewport.name}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await installTauriMocks(page);
    await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/appearance");
    await expect(page.getByRole("heading", { name: "外观", exact: true })).toBeVisible();
    expect(
      await page
        .locator(".settings-main")
        .evaluate((element) => element.scrollWidth <= element.clientWidth),
    ).toBe(true);
    await expect(page).toHaveScreenshot(`production-appearance-${viewport.name}.png`, {
      animations: "disabled",
    });
  });
}

test("production theme dropdown matches the compact floating style", async ({ page }) => {
  await page.setViewportSize({ width: 1855, height: 1343 });
  await installTauriMocks(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/appearance");
  const lightEditor = page.locator(".theme-profile-card").nth(0);
  await lightEditor.locator('[data-component="select-trigger"]').nth(0).click();
  await expect(page.getByRole("option")).toHaveCount(9);
  const contentBox = await page.locator('[data-component="select-content"]').boundingBox();
  const firstOptionBox = await page.getByRole("option").first().boundingBox();
  expect(contentBox).not.toBeNull();
  expect(firstOptionBox).not.toBeNull();
  expect(firstOptionBox!.x - contentBox!.x).toBeLessThan(16);
  await expect(page).toHaveScreenshot("production-theme-dropdown-1855x1343.png", {
    animations: "disabled",
  });
});

test("production navigation and theme mode remain interactive", async ({ page }) => {
  await installTauriMocks(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
  await page.getByRole("button", { name: /连接大语言模型/ }).click();
  await expect(page.getByRole("heading", { name: "大语言模型" })).toBeVisible();
  await page.getByRole("button", { name: "外观" }).click();
  await page.getByRole("button", { name: "浅色" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-color-scheme", "light");
});

test("production menus contain only implemented entries and legacy routes normalize", async ({
  page,
}) => {
  await installTauriMocks(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
  await expect(page.getByRole("button", { name: "新建任务" })).toBeVisible();
  for (const removed of ["拉取请求", "已安排", "插件"]) {
    await expect(page.getByRole("button", { name: removed, exact: true })).toHaveCount(0);
  }

  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/plugins");
  await expect(page.getByRole("heading", { name: "通用", exact: true, level: 1 })).toBeVisible();
  await expect(page.locator(".settings-nav button")).toHaveCount(6);
  for (const entry of ["通用", "外观", "语音", "配置", "宠物", "交互"]) {
    await expect(page.locator(".settings-nav").getByRole("button", { name: entry })).toBeVisible();
  }
});

test("appearance controls update runtime tokens and support wheel and keyboard", async ({
  page,
}) => {
  await installTauriMocks(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/appearance");
  const root = page.locator("html");
  const scroll = page.locator(".settings-scroll");

  await scroll.hover();
  await page.mouse.wheel(0, 900);
  await expect.poll(() => scroll.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);

  await page
    .locator('[data-component="settings-row"]', { hasText: "按钮使用指针光标" })
    .locator('[data-component="switch-root"]')
    .click();
  await expect(root).toHaveAttribute("data-pointer-cursor", "on");

  const motion = page.locator(".appearance-preferences [data-component=select-trigger]");
  await motion.click();
  await page.getByRole("option", { name: "已启用" }).click();
  await expect(root).toHaveAttribute("data-reduced-motion", "on");

  const uiSize = page
    .locator('[data-component="range-field"]', { hasText: "界面字号" })
    .locator('[data-component="range-thumb"]');
  await uiSize.focus();
  await page.keyboard.press("ArrowUp");
  await expect(root).toHaveCSS("--font-size-md", "15px");

  await page.getByRole("button", { name: "+/-", exact: true }).click();
  await expect(root).toHaveAttribute("data-diff-markers", "signs");

  const sidebarSwitches = page
    .locator('[data-component="settings-row"]', { hasText: "半透明侧栏" })
    .locator('[data-component="switch-root"]');
  const sidebar = page.locator(".settings-sidebar");
  const translucentBackground = await sidebar.evaluate(
    (element) => getComputedStyle(element).backgroundImage,
  );
  await sidebarSwitches.nth(1).click();
  await expect(root).toHaveAttribute("data-translucent-sidebar", "off");
  await expect
    .poll(() => sidebar.evaluate((element) => getComputedStyle(element).backgroundImage))
    .not.toBe(translucentBackground);

  const darkEditor = page.locator(".theme-profile-card").nth(1);
  const contrast = darkEditor
    .locator('[data-component="range-field"]', { hasText: "界面对比度" })
    .locator('[data-component="range-thumb"]');
  const panelBefore = await root.evaluate((element) =>
    element.style.getPropertyValue("--appearance-panel"),
  );
  await contrast.focus();
  await page.keyboard.press("End");
  await expect
    .poll(() => root.evaluate((element) => element.style.getPropertyValue("--appearance-panel")))
    .not.toBe(panelBefore);
  await expect(root).toHaveCSS("--appearance-contrast", "100");

  const darkBackground = darkEditor.locator('input[type="text"][aria-label="背景色"]');
  const darkForeground = darkEditor.locator('input[type="text"][aria-label="前景色"]');
  await darkBackground.fill("#101010");
  await darkForeground.fill("#111111");
  await expect(page.getByText(/WCAG AA 4\.5:1/)).toBeVisible();
  await expect(root).toHaveCSS("--appearance-background", "#101010");
  await expect(root).toHaveCSS("--appearance-foreground", "#111111");
  await expect(page.getByText("已保存", { exact: true })).toBeVisible();
});

test("built-in themes and font presets switch the complete interface", async ({ page }) => {
  await installTauriMocks(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/appearance");
  const root = page.locator("html");
  const lightEditor = page.locator(".theme-profile-card").nth(0);

  await lightEditor.locator('[data-component="select-trigger"]').nth(0).click();
  await expect(page.getByRole("option")).toHaveCount(9);
  await page.getByRole("option", { name: "Catppuccin" }).click();
  await expect(root).toHaveAttribute("data-color-scheme", "light");
  await expect(root).toHaveCSS("--appearance-background", "#EFF1F5");

  const uiFontRow = lightEditor.locator('[data-component="settings-row"]', {
    hasText: "界面字体栈",
  });
  await uiFontRow.locator('[data-component="select-trigger"]').click();
  await page.getByRole("option", { name: "Microsoft YaHei" }).click();
  await expect
    .poll(() => root.evaluate((element) => element.style.getPropertyValue("--font-ui")))
    .toContain("Microsoft YaHei");

  const codeFontRow = lightEditor.locator('[data-component="settings-row"]', {
    hasText: "代码字体栈",
  });
  await codeFontRow.locator('[data-component="select-trigger"]').click();
  await page.getByRole("option", { name: "Consolas" }).click();
  await expect
    .poll(() => root.evaluate((element) => element.style.getPropertyValue("--font-code")))
    .toBe("Consolas, monospace");
});

test("theme import, copy, delete, and built-in reset complete through native commands", async ({
  page,
}) => {
  await installTauriMocks(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/appearance");
  const editors = page.locator(".theme-profile-card");
  const lightEditor = editors.nth(0);

  await lightEditor.getByRole("button", { name: "导入" }).click();
  await expect(lightEditor.locator('[data-component="select-trigger"]').nth(0)).toContainText(
    "Imported light",
  );
  await lightEditor.getByRole("button", { name: "复制 JSON" }).click();
  await expect(page.getByText("主题 JSON 已复制到系统剪贴板。")).toBeVisible();

  await lightEditor.getByRole("button", { name: "主题操作" }).click();
  await page.getByRole("menuitem", { name: "删除" }).click();
  await expect(page.getByRole("dialog", { name: "删除自定义主题" })).toBeVisible();
  await page.getByRole("dialog").getByRole("button", { name: "确认" }).click();
  await expect(lightEditor.locator('[data-component="select-trigger"]').nth(0)).toContainText(
    "Codex",
  );

  const darkEditor = editors.nth(1);
  const darkAccent = darkEditor.locator('input[type="text"][aria-label="强调色"]');
  await darkAccent.fill("#FF0000");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (
          window as unknown as Window & {
            __HACHIMI_TEST_CALLS__: Array<{
              command: string;
              args: { settings?: { appearance?: { themes?: Array<{ accent?: string }> } } };
            }>;
          }
        ).__HACHIMI_TEST_CALLS__.some(
          (call) =>
            call.command === "update_settings" &&
            call.args.settings?.appearance?.themes?.some((profile) => profile.accent === "#FF0000"),
        ),
      ),
    )
    .toBe(true);
  await darkEditor.getByRole("button", { name: "主题操作" }).click();
  await page.getByRole("menuitem", { name: "恢复默认" }).click();
  await expect(page.getByRole("dialog", { name: "恢复内置主题" })).toBeVisible();
  await page.getByRole("dialog").getByRole("button", { name: "确认" }).click();
  await expect(darkAccent).toHaveValue("#2EA8FF");

  const commands = await page.evaluate(() =>
    (
      window as unknown as Window & {
        __HACHIMI_TEST_CALLS__: Array<{ command: string }>;
      }
    ).__HACHIMI_TEST_CALLS__.map((call) => call.command),
  );
  expect(commands).toEqual(
    expect.arrayContaining([
      "import_theme_profile",
      "copy_theme_profile",
      "delete_theme_profile",
      "reset_theme_profile",
    ]),
  );
});

test("appearance save failures roll the preview back to the confirmed settings", async ({
  page,
}) => {
  await installTauriMocks(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/appearance");
  await page.evaluate(() => {
    (
      window as unknown as Window & {
        __HACHIMI_TEST_STATE__: { failNextUpdate: boolean };
      }
    ).__HACHIMI_TEST_STATE__.failNextUpdate = true;
  });
  await page
    .locator('[data-component="settings-row"]', { hasText: "按钮使用指针光标" })
    .locator('[data-component="switch-root"]')
    .click();
  await expect(page.locator("html")).toHaveAttribute("data-pointer-cursor", "off");
  await expect(page.getByText("保存失败，已回滚", { exact: true })).toBeVisible();
});

test("model, voice, and pet settings use their live command-backed controls", async ({ page }) => {
  await installTauriMocks(page);
  await installMotionLabAssets(page);

  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/llm");
  await expect(page.getByRole("textbox", { name: "接口地址" })).toHaveValue(
    "http://localhost:11434/v1",
  );
  await page.getByRole("textbox", { name: "模型名称" }).fill("gpt-test");
  await page.getByRole("button", { name: "仅保存" }).click();
  await expect(page.getByText("设置已保存。")).toBeVisible();

  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/voice");
  await expect(page.getByText("SenseVoice-Small INT8", { exact: true })).toBeVisible();
  await expect(page.getByText("内置 · 可离线使用", { exact: true })).toBeVisible();
  await expect(page.getByText("DirectML", { exact: true }).first()).toBeVisible();
  const speed = page
    .locator('[data-component="range-field"]', { hasText: "语速（百分比）" })
    .locator('[data-component="range-thumb"]');
  await speed.focus();
  await page.keyboard.press("ArrowUp");
  await expect(page.getByText("VITS 语音设置已保存。")).toBeVisible();
  await page
    .locator('[data-component="settings-row"]', { hasText: "桌宠静音" })
    .locator('[data-component="switch-root"]')
    .click();
  await page.getByRole("textbox", { name: "资源名称" }).fill("Melo 中英女声");
  await page.getByRole("button", { name: "选择模型并检测" }).click();
  const voiceInspection = page.getByRole("dialog", { name: "VITS 模型检测结果" });
  await expect(voiceInspection.getByText("vits-melo", { exact: true })).toBeVisible();
  await expect(
    voiceInspection.getByText("vits-melo-tts-zh_en/model.onnx", { exact: true }),
  ).toBeVisible();
  await voiceInspection.locator('[data-component="switch-root"]').click();
  await voiceInspection.getByRole("button", { name: "确认导入" }).click();
  await expect(page.getByText("Melo 中英女声", { exact: true })).toBeVisible();

  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/avatar");
  await expect(page.getByText("Mimi", { exact: true })).toBeVisible();
  await expect(page.getByText(/新导入仅接受.*VRM 0.x \/ 1.0/)).toBeVisible();
  await page.getByRole("textbox", { name: "资源名称" }).fill("Luna");
  await page.getByRole("button", { name: "选择模型并检测" }).click();
  const inspection = page.getByRole("dialog", { name: "模型兼容性检测" });
  await expect(inspection.getByText("Runtime Ready", { exact: true }).nth(0)).toBeVisible();
  await expect(inspection.getByText(/标准动作、神态、视线、口型/)).toBeVisible();
  await inspection.getByRole("button", { name: "确认导入" }).click({ force: true });
  await expect(page.getByText("Luna", { exact: true })).toBeVisible();
  await expect(page.locator(".avatar-card-preview canvas")).toHaveCount(2);

  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/motion");
  await expect(page.getByRole("heading", { name: "交互", exact: true })).toBeVisible();
  await expect(page.getByText("标准待机", { exact: true }).first()).toBeVisible();
  await expect(page.locator(".motion-preview-stage canvas")).toHaveCount(1);
  const builtinMotion = page.locator(".motion-entry-card", { hasText: "标准待机" });
  await expect(builtinMotion.getByText("内置锁定", { exact: true })).toBeVisible();
  await expect(builtinMotion.getByRole("button", { name: "删除" })).toHaveCount(0);
  await page.getByRole("button", { name: "上传 VRMA" }).click();
  const motionDialog = page.getByRole("dialog", { name: "上传用户动作" });
  await expect(motionDialog.getByText(/30 fingers/)).toBeVisible();
  await motionDialog.getByLabel("名称").fill("My Wave");
  await motionDialog.getByRole("button", { name: "保存" }).click();
  const userMotionCard = page.locator(".motion-entry-card", { hasText: "My Wave" });
  await expect(userMotionCard).toBeVisible();
  await userMotionCard.getByRole("button", { name: "删除" }).click();
  await page
    .getByRole("dialog", { name: "删除用户动作" })
    .getByRole("button", { name: "删除" })
    .click();
  await expect(userMotionCard).not.toBeVisible();

  await page.getByRole("tab", { name: "互动" }).click();
  await expect(page.getByRole("button", { name: /头顶/ })).toBeVisible();
  await expect(page.getByText("点头同意", { exact: true }).first()).toBeVisible();
});

test("motion settings keep one motion per region and import an optional binding", async ({
  page,
}) => {
  await installTauriMocks(page);
  await installMotionLabAssets(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/motion");

  await page.getByRole("tab", { name: "互动" }).click();
  await page.getByRole("button", { name: /头顶/ }).click();
  const boundMotionField = page.locator('[data-component="form-label"]', {
    hasText: "绑定动作",
  });
  await boundMotionField.locator("..").locator('[data-component="select-trigger"]').click();
  await page
    .locator('[data-component="select-content"]')
    .last()
    .locator('[data-component="select-item"]', { hasText: "标准待机" })
    .click();
  await expect(page.getByText("标准待机", { exact: true }).first()).toBeVisible();
  await page.getByRole("button", { name: "恢复此区域默认" }).click();

  await page.getByRole("tab", { name: "动作" }).click();
  await page.getByRole("button", { name: "上传 VRMA" }).click();
  const dialog = page.getByRole("dialog", { name: "上传用户动作" });
  await dialog.getByLabel("名称").fill("Face Reaction");
  const regionLabel = dialog.locator('[data-component="form-label"]', {
    hasText: "绑定互动区域",
  });
  await regionLabel.locator("..").locator('[data-component="select-trigger"]').click();
  await page
    .locator('[data-component="select-content"]')
    .last()
    .locator('[data-component="select-item"]', { hasText: /^脸部$/ })
    .click();
  await dialog.getByRole("button", { name: "保存" }).click();
  await expect(page.locator(".motion-entry-card", { hasText: "Face Reaction" })).toBeVisible();

  const calls = await page.evaluate(
    () =>
      (
        window as unknown as Window & {
          __HACHIMI_TEST_CALLS__: Array<{
            command: string;
            args: {
              request?: {
                interactionRegion?: string;
                motionId?: string | null;
              };
            };
          }>;
        }
      ).__HACHIMI_TEST_CALLS__,
  );
  expect(
    calls.some(
      (call) =>
        call.command === "set_interaction_motion_binding" &&
        call.args.request?.motionId === motionEntry.id,
    ),
  ).toBe(true);
  expect(
    calls.some(
      (call) =>
        call.command === "commit_motion_import" && call.args.request?.interactionRegion === "face",
    ),
  ).toBe(true);
  expect(calls.some((call) => call.command === "reset_motion_binding")).toBe(true);
});

test("motion settings share one responsive preview across both tabs", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await installTauriMocks(page);
  await installMotionLabAssets(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/motion");

  const previewCanvas = page.locator(".motion-preview-stage canvas");
  await expect(previewCanvas).toHaveCount(1);
  await expect(page.getByText("标准待机", { exact: true }).first()).toBeVisible();
  await expect(page).toHaveScreenshot("production-motion-settings-motions-1280x800.png", {
    animations: "disabled",
    mask: [previewCanvas],
    maskColor: "#313236",
  });

  await page.getByRole("tab", { name: "互动" }).click();
  await expect(previewCanvas).toHaveCount(1);
  await expect(page.getByRole("button", { name: /头顶/ })).toBeVisible();
  await expect(page).toHaveScreenshot("production-motion-settings-interactions-1280x800.png", {
    animations: "disabled",
    mask: [previewCanvas],
    maskColor: "#313236",
  });

  await page.setViewportSize({ width: 960, height: 640 });
  await page.getByRole("tab", { name: "动作" }).click();
  const previewBox = await page.locator(".motion-settings-preview").boundingBox();
  const browserBox = await page.locator(".motion-settings-browser").boundingBox();
  expect(previewBox).not.toBeNull();
  expect(browserBox).not.toBeNull();
  expect(previewBox!.y).toBeLessThan(browserBox!.y);
  await expect(previewCanvas).toHaveCount(1);
  await expect(page).toHaveScreenshot("production-motion-settings-motions-960x640.png", {
    animations: "disabled",
    mask: [previewCanvas],
    maskColor: "#313236",
  });
});

test("Motion Library Lab previews a catalog VRMA with finger diagnostics", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await installTauriMocks(page);
  await installMotionLabAssets(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=developer/motion-lab");
  await expect(page.getByRole("heading", { name: "动作库实验室" })).toBeVisible();
  await expect(page.getByLabel("VRMA 动作")).toHaveValue(motionEntry.id);
  await page.getByRole("button", { name: "暂停" }).click();
  await page.getByLabel("动作时间").fill("1100");
  await expect(page.getByLabel("动作时间")).toHaveValue("1100");
  await expect(
    page.locator(".motion-lab-metrics > div", { hasText: "活动骨骼" }).locator("strong"),
  ).not.toHaveText("0");
  await expect(page.getByText("30", { exact: true }).first()).toBeVisible();
  await expect(page.getByText("Root 轨迹", { exact: true })).toBeVisible();
  await expect(page.getByText("足部接触", { exact: true })).toBeVisible();
  await expect(page.getByText("接触时间线", { exact: true })).toBeVisible();
  await expect(page.getByText(/° · .* m/)).toBeVisible();
  await page.getByLabel("播放速度").fill("1.5");
  await expect(page.getByLabel("播放速度")).toHaveValue("1.5");
  await expect(page).toHaveScreenshot("production-motion-lab-1280x800.png", {
    animations: "disabled",
    mask: [
      page.locator(".motion-lab-stage canvas"),
      page.locator(".motion-lab-metrics > div", { hasText: "Solve" }).locator("strong"),
    ],
    maskColor: "#313236",
  });
});

test("resetting all local data requires explicit confirmation", async ({ page }) => {
  await installTauriMocks(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/general");
  await page.getByRole("button", { name: "重置全部本地数据" }).click();
  const dialog = page.getByRole("dialog", { name: "重置 Hachimi" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "取消" }).click();
  await expect(dialog).not.toBeVisible();

  await page.getByRole("button", { name: "重置全部本地数据" }).click();
  await dialog.getByRole("button", { name: "确认" }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (
          window as unknown as Window & {
            __HACHIMI_TEST_CALLS__: Array<{ command: string }>;
          }
        ).__HACHIMI_TEST_CALLS__.some((call) => call.command === "reset_local_data"),
      ),
    )
    .toBe(true);
});

test("production home and appearance have no new WCAG A or AA violations", async ({ page }) => {
  await installTauriMocks(page);
  for (const route of ["home", "settings/appearance"]) {
    await page.goto(`http://127.0.0.1:1420/workbench.html?route=${route}`);
    await expect(page.getByText("Hachimi", { exact: true }).first()).toBeVisible();
    const result = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
      // Kobalte exposes one focusable composite widget while keeping its
      // native form input visually hidden. Axe treats that implementation
      // detail as nested/aria-hidden focus even though it is removed from the
      // tab order and announced through the composite control.
      .disableRules(["aria-hidden-focus", "nested-interactive"])
      .analyze();
    expect(result.violations).toEqual([]);
  }
});
