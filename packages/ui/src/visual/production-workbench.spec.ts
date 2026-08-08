import { expect, test, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { resolve } from "node:path";
import {
  appearance,
  directTerminalFixture,
  hostSettingsMocks,
  initialSettings,
  installMotionLabAssets,
  motionEntry,
  runtimeAssessment,
  runtimeMocks,
  taskRunFixtures,
  taskScheduleFixtures,
  workbenchEnvironmentFixture,
  workspaceDiffFixture,
  workspaceFileChunkFixture,
  workspaceGitFixture,
} from "./production-workbench-fixtures";
import { installEnvironmentSummaryVisualTests } from "./environment-summary-visual";
import { assertPlatformIntegrationsVisualMatrix } from "./platform-integrations-visual";
import { assertPermissionTones, installSessionScrollVisualTest } from "./session-scroll-visual";
import { installSettingsLayoutVisualTests } from "./settings-layout-visual";
export async function installTauriMocks(
  page: Page,
  withComposerData = false,
  schedulerEnabled = false,
  withSessionData = false,
  themeMode: "light" | "dark" | "system" = "dark",
  gateMode: "approval" | "plan" | "user_input" = "approval",
) {
  await page.addInitScript(
    ({
      appearance,
      directTerminalFixture,
      hostSettingsMocks,
      initialSettings,
      motionEntry,
      runtimeAssessment,
      runtimeMocks,
      taskRunFixtures,
      taskScheduleFixtures,
      workbenchEnvironmentFixture,
      workspaceDiffFixture,
      workspaceFileChunkFixture,
      workspaceGitFixture,
      withComposerData,
      schedulerEnabled,
      withSessionData,
      themeMode,
      gateMode,
    }) => {
      type MockSettings = Omit<typeof initialSettings, "theme"> & { theme: typeof themeMode };
      let settings: MockSettings = { ...initialSettings, theme: themeMode };
      let voiceRuntime = structuredClone(runtimeMocks.voiceRuntime);
      let speechRecognition = structuredClone(runtimeMocks.speechRecognition);
      let voices = structuredClone(runtimeMocks.voices);
      let avatars = structuredClone(runtimeMocks.avatars);
      let motions = structuredClone(runtimeMocks.motions);
      const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
      const testState = { failNextUpdate: false };
      const taskSchedules = structuredClone(taskScheduleFixtures);
      const taskRuns = structuredClone(taskRunFixtures);
      const session = {
        id: "session-ui-unification",
        context: {
          kind: "project",
          project_id: "project-hachimi",
          checkout_id: "checkout-ui-unification",
        },
        entryProfile: "workbench",
        title: "统一前端视觉规范与组件样式",
        archived: false,
        pinned: false,
        parentSessionId: null,
        sourceRunId: null,
        createdAtMs: 1_774_184_400_000,
        updatedAtMs: 1_774_184_520_000,
      };
      const run = {
        id: "run-ui-unification",
        sessionId: session.id,
        status:
          gateMode === "user_input"
            ? "waiting_user_input"
            : gateMode === "plan"
              ? "succeeded"
              : "waiting_approval",
        purpose: "task",
        generation: 2,
        configuration: {},
        requestedCapabilities: {},
        negotiatedCapabilities: {},
        capabilityDegradations: [],
        failureCode: null,
        createdAtMs: 1_774_184_400_000,
        updatedAtMs: 1_774_184_520_000,
      };
      const transcript = [
        {
          id: "item-user",
          sessionId: session.id,
          runId: run.id,
          sequence: 1,
          kind: "user",
          status: "completed",
          payload: {
            type: "user",
            data: {
              text: "分析并统一前端各组件的颜色、排版、字号和交互状态。",
              attachment_ids: [],
            },
          },
          relations: {},
          createdAtMs: 1_774_184_400_000,
        },
        {
          id: "item-assistant",
          sessionId: session.id,
          runId: run.id,
          sequence: 2,
          kind: "assistant",
          status: "completed",
          payload: {
            type: "assistant",
            data: {
              text: "已将 Quiet Graphite 令牌、设置行和 Composer 提取为共享实现，正在核对正式页面。",
            },
          },
          relations: {},
          createdAtMs: 1_774_184_460_000,
        },
        {
          id: "item-tool",
          sessionId: session.id,
          runId: run.id,
          sequence: 3,
          kind: "tool_execution",
          status: "completed",
          payload: {
            type: "tool_execution",
            data: {
              tool_call_id: "tool-call-ui-unification",
              name: "apply_patch",
              arguments: { patch: "*** Begin Patch" },
              step_revision: 1,
              tool_plan_hash: "ui-tool-plan",
              registry_revision: "ui-tool-registry",
              result: {
                status: "succeeded",
                modelContent: "检查组件契约：42 个共享模式通过。",
                structuredContent: { passed: 42 },
                stableResultCode: "component_contracts_passed",
              },
            },
          },
          relations: {},
          createdAtMs: 1_774_184_490_000,
        },
      ] as Array<Record<string, unknown>>;
      const approval = {
        id: "approval-ui-unification",
        sessionId: session.id,
        runId: run.id,
        toolCallId: "tool-call-ui-unification",
        runGeneration: run.generation,
        status: "pending",
        action: "write_file",
        resource: "packages/ui/src/styles",
        parameterHash: "ui-contract",
        riskSummary: "将更新共享组件样式并同步到正式页面。",
        targetHost: "local",
        requiredScopes: ["workspace.write"],
        grantScope: "once",
        usesRemaining: 1,
        requesterPrincipal: "hachimi-agent",
        resolvedBy: null,
        expiresAtMs: null,
        createdAtMs: 1_774_184_520_000,
        resolvedAtMs: null,
      };
      const proposedPlan = {
        id: "plan-ui-unification",
        sessionId: session.id,
        sourceRunId: run.id,
        sourceItemId: "item-plan",
        revision: 2,
        title: "Workbench Codex 对齐计划",
        goal: "统一 Workbench 消息、摘要、终端和右侧查看器",
        contentMarkdown: "# Workbench Codex 对齐计划\n\n完整实施消息、摘要、终端和 Inspector。",
        createdAtMs: 1_774_184_520_000,
      };
      const planConfirmation = {
        planId: proposedPlan.id,
        status: "pending",
        acceptedRunId: null,
        resolvedAtMs: null,
      };
      const userInput = {
        id: "input-ui-unification",
        sessionId: session.id,
        runId: run.id,
        runGeneration: run.generation,
        itemId: "item-user-input",
        questions: [
          {
            id: "scope",
            header: "对齐范围",
            prompt: "本轮应优先完成哪一部分？",
            options: [
              { label: "完整工作台", value: "all", description: "消息、摘要、终端和 Inspector" },
              { label: "消息时间线", value: "timeline", description: "优先收敛流式步骤" },
              { label: "面板布局", value: "panels", description: "优先收敛摘要与查看器" },
            ],
            secret: false,
            autoResolutionMs: 120000,
            defaultAnswer: "all",
          },
          {
            id: "density",
            header: "界面密度",
            prompt: "执行过程采用哪种信息密度？",
            options: [
              { label: "Codex 紧凑", value: "compact", description: "推荐，显示更多运行步骤" },
              { label: "标准", value: "normal", description: "保留更多留白" },
            ],
            secret: false,
            autoResolutionMs: null,
            defaultAnswer: "compact",
          },
        ],
        status: "pending",
        expiresAtMs: 1_774_184_640_000,
        createdAtMs: 1_774_184_520_000,
        resolvedAtMs: null,
        resolvedBy: null,
      };
      if (gateMode === "plan") {
        transcript.push({
          id: "item-plan",
          sessionId: session.id,
          runId: run.id,
          sequence: 4,
          kind: "plan",
          status: "completed",
          payload: {
            type: "plan",
            data: {
              text: proposedPlan.contentMarkdown,
            },
          },
          relations: {},
          createdAtMs: 1_774_184_520_000,
        });
      }
      if (gateMode === "user_input") {
        transcript.push({
          id: userInput.itemId,
          sessionId: session.id,
          runId: run.id,
          sequence: 4,
          kind: "user_input_request",
          status: "in_progress",
          payload: {
            type: "user_input_request",
            data: {
              request_id: userInput.id,
              questions: userInput.questions,
              display_answers: [],
            },
          },
          relations: { userInputRequestId: userInput.id },
          createdAtMs: userInput.createdAtMs,
        });
      }
      const attachment = {
        id: "attachment-visual-notes",
        contentHash: "attachment-visual-notes-hash",
        originalName: "notes.txt",
        mimeType: "text/plain",
        byteSize: 15,
        createdAtMs: 1_774_184_520_000,
      };
      const sessionSnapshot = {
        session,
        checkout: {
          id: "checkout-ui-unification",
          projectId: "project-hachimi",
          kind: "local",
          path: "D:\\workspace\\rust\\hachimi-code",
          baseRevision: null,
          headRevision: "1234567890abcdef",
          status: "ready",
          pinned: false,
          createdAtMs: 1_774_184_400_000,
          updatedAtMs: 1_774_184_520_000,
        },
        runs: [run],
        events: [],
        transcript,
        attachments: [attachment],
        pendingApprovals: gateMode === "approval" ? [approval] : [],
        planDocuments: gateMode === "plan" ? [proposedPlan] : [],
        planConfirmations: gateMode === "plan" ? [planConfirmation] : [],
        executionPlans: [],
        artifacts: [],
        agentTasks: [],
        runSummaries: [],
        browserSessions: [
          {
            id: "browser-ui-unification",
            profileKind: "isolated",
            ownerSessionId: session.id,
            ownerRunId: run.id,
            runGeneration: run.generation,
            origin: "https://learn.chatgpt.com",
            currentUrl: "https://learn.chatgpt.com/docs/environments/git-worktrees",
            taskTabGroup: "visual-test",
            revision: 3,
            status: "ready",
            createdAtMs: 1_774_184_500_000,
          },
        ],
        browserAutomationLeases: [],
        externalBrowserObservations: [],
        hostAccessRequests: [],
        computerControlSessions: [],
        sources: workbenchEnvironmentFixture.sources,
      };
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
          if (command === "initialize_agent_control") {
            return {
              protocolVersion: 31,
              acceptedFeatures: ["workbench", "workspace_tools"],
              sandbox: { osEnforced: true },
            };
          }
          if (command === "list_workbench_projects") {
            return withComposerData
              ? [
                  {
                    id: "project-hachimi",
                    displayName: "hachimi-code",
                    rootPath: "D:\\workspace\\rust\\hachimi-code",
                    gitRoot: "D:\\workspace\\rust\\hachimi-code",
                    trusted: true,
                    createdAtMs: 1,
                    updatedAtMs: 1,
                  },
                  {
                    id: "project-notes",
                    displayName: "notes",
                    rootPath: "D:\\workspace\\notes",
                    gitRoot: null,
                    trusted: true,
                    createdAtMs: 2,
                    updatedAtMs: 2,
                  },
                ]
              : [];
          }
          if (command === "list_workbench_sessions") {
            return withSessionData ? [{ session, latestRun: run, latestTerminalRun: null }] : [];
          }
          if (command === "search_agent_sessions") {
            return { items: withSessionData ? [session] : [], nextCursor: null };
          }
          if (command === "get_workbench_session") return sessionSnapshot;
          if (command === "get_workbench_environment") return workbenchEnvironmentFixture;
          if (command === "get_workbench_project_tool_context") return sessionSnapshot;
          if (command === "resume_agent_session") {
            return {
              session,
              activeRun: run,
              transcript,
              pendingApprovals: gateMode === "approval" ? [approval] : [],
              pendingUserInputs: gateMode === "user_input" ? [userInput] : [],
              usageSnapshot: null,
              snapshotSequence: 3,
              previousTranscriptCursor: null,
            };
          }
          if (command === "subscribe_agent_events") {
            return {
              subscription: {
                id: "subscription-ui-unification",
                sessionId: session.id,
                clientId: "visual-test",
                afterSequence: 3,
              },
              catchUp: [],
            };
          }
          if (command === "unsubscribe_agent_events" || command === "unwatch_workspace_files") {
            return true;
          }
          if (command === "list_workspace_files") {
            return {
              path: "",
              entries: [
                {
                  path: "packages",
                  name: "packages",
                  kind: "directory",
                  byteSize: null,
                  modifiedAtMs: 1_774_184_500_000,
                  hidden: false,
                  hasChildren: true,
                  gitStatus: null,
                },
                {
                  path: "README.md",
                  name: "README.md",
                  kind: "file",
                  byteSize: 6_144,
                  modifiedAtMs: 1_774_184_500_000,
                  hidden: false,
                  hasChildren: false,
                  gitStatus: "modified",
                },
              ],
              nextCursor: null,
              etag: "workspace-ui-unification",
            };
          }
          if (command === "read_workspace_file_chunk") return workspaceFileChunkFixture;
          if (command === "watch_workspace_files") {
            return {
              id: "watch-ui-unification",
              sessionId: session.id,
              checkoutId: "checkout-ui-unification",
              path: "",
              generation: 1,
            };
          }
          if (command === "get_workspace_git") return workspaceGitFixture;
          if (command === "get_workspace_diff")
            return { ...workspaceDiffFixture, scope: args.scope };
          if (command === "list_schedules") return schedulerEnabled ? taskSchedules : [];
          if (command === "list_task_runs") return schedulerEnabled ? taskRuns : [];
          if (
            command === "list_schedule_event_receipts" ||
            command === "list_run_recoveries" ||
            command === "list_processes" ||
            command === "list_reviews"
          ) {
            return [];
          }
          if (command === "spawn_process") return directTerminalFixture;
          if (command === "read_process") {
            return {
              process: directTerminalFixture,
              chunks: [],
              nextSequence: 0,
              closed: false,
            };
          }
          if (command === "resize_process") return null;
          const hostSettingsMock = (hostSettingsMocks as Record<string, unknown>)[command];
          if (hostSettingsMock !== undefined) return hostSettingsMock;
          if (command === "list_mcp_servers") return [];
          if (command === "inspect_project_git" || command === "refresh_project_git") {
            return {
              projectId: "project-hachimi",
              gitRoot: "D:\\workspace\\rust\\hachimi-code",
              state: { kind: "ready", branch: "main", head: "1234567890abcdef" },
              observedAtMs: 1,
            };
          }
          if (command === "list_project_git_refs") {
            return [
              { name: "main", revision: "1234567890abcdef", remote: false, current: true },
              {
                name: "feature/composer",
                revision: "abcdef1234567890",
                remote: false,
                current: false,
              },
              { name: "origin/main", revision: "fedcba0987654321", remote: true, current: false },
            ];
          }
          if (command === "list_skills") {
            return withComposerData
              ? [
                  {
                    id: "skill-documents",
                    name: "Documents",
                    qualifiedName: "Documents",
                    description: "创建和编辑文档工件",
                    enabled: true,
                    contentHash: "documents-hash",
                    treeRevision: "documents-revision",
                    diagnostics: [],
                    updatedAtMs: 1,
                  },
                  {
                    id: "skill-pdf",
                    name: "PDF",
                    qualifiedName: "PDF",
                    description: "读取、创建和验证 PDF 文件",
                    enabled: true,
                    contentHash: "pdf-hash",
                    treeRevision: "pdf-revision",
                    diagnostics: [],
                    updatedAtMs: 2,
                  },
                  {
                    id: "skill-spreadsheets",
                    name: "Spreadsheets",
                    qualifiedName: "Spreadsheets",
                    description: "创建和编辑电子表格",
                    enabled: true,
                    contentHash: "spreadsheets-hash",
                    treeRevision: "spreadsheets-revision",
                    diagnostics: [],
                    updatedAtMs: 3,
                  },
                ]
              : [];
          }
          if (command === "import_workbench_attachment") return attachment;
          if (command === "read_workbench_attachment") {
            return {
              attachment,
              utf8Text: "reference notes",
              dataUrl: null,
              truncated: false,
            };
          }
          if (command === "get_bootstrap_state") {
            return {
              protocolVersion: 31,
              windowKind: "workbench",
              locale: "zh-CN",
              theme: themeMode,
              appearance,
              alwaysOnTop: true,
              featureFlags: {
                workbench: true,
                motionLab: true,
                workspaceTools: withComposerData,
                browserControl: false,
                computerObserve: false,
                computerControl: false,
                remoteTts: false,
                remoteGateway: false,
                pluginRuntime: true,
                localGateway: true,
                mcpRuntime: false,
                scheduler: schedulerEnabled,
                runtimeFeatures: {
                  runRecovery: true,
                  providerExtensions: true,
                  providerRemoteContext: true,
                  multiAgent: true,
                  gitRemoteMutations: true,
                  pluginRuntime: true,
                  enterpriseIntegrations: true,
                },
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
            settings = args.settings as MockSettings;
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
    {
      appearance,
      directTerminalFixture,
      hostSettingsMocks,
      initialSettings,
      motionEntry,
      runtimeAssessment,
      runtimeMocks,
      taskRunFixtures,
      taskScheduleFixtures,
      workbenchEnvironmentFixture,
      workspaceDiffFixture,
      workspaceFileChunkFixture,
      workspaceGitFixture,
      withComposerData,
      schedulerEnabled,
      withSessionData,
      themeMode,
      gateMode,
    },
  );
}

installSessionScrollVisualTest(installTauriMocks);
installEnvironmentSummaryVisualTests(installTauriMocks);
installSettingsLayoutVisualTests(installTauriMocks);
test("production task center uses cards and focused dialogs", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.clock.setFixedTime(new Date("2026-07-26T15:00:00.000Z"));
  await installTauriMocks(page, true, true);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
  await page.getByTestId("workbench-task-tab").click();
  await expect(page.getByTestId("workbench-task-center")).toBeVisible();
  await expect(page.locator(".workbench-toolbar")).toHaveCount(0);
  await expect(page.getByTestId("task-schedule-card")).toHaveCount(2);
  expect((await new AxeBuilder({ page }).include(".task-center").analyze()).violations).toEqual([]);
  await expect(page).toHaveScreenshot("production-task-center-cards-1280x800.png", {
    animations: "disabled",
  });
  await page.getByTestId("task-create-toggle").click();
  await expect(page.getByTestId("task-name")).toBeVisible();
  expect((await new AxeBuilder({ page }).include('[role="dialog"]').analyze()).violations).toEqual(
    [],
  );
  await expect(page).toHaveScreenshot("production-task-center-1280x800.png", {
    animations: "disabled",
  });
  await page.getByRole("button", { name: /关闭|Close/ }).click();
  await page.getByTestId("task-history").first().click();
  await expect(page.getByTestId("task-run-history")).toBeVisible();
  await expect(page).toHaveScreenshot("production-task-history-1280x800.png", {
    animations: "disabled",
  });
  await page.getByRole("button", { name: /关闭|Close/ }).click();
  await page.setViewportSize({ width: 720, height: 640 });
  await expect(page.locator("html")).toHaveJSProperty("scrollWidth", 720);
  await expect(page).toHaveScreenshot("production-task-center-cards-720x640.png", {
    animations: "disabled",
  });
});
test("production composer popovers dismiss outside and create visual Skill references", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await installTauriMocks(page, true);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
  await expect(page.getByRole("heading", { name: /hachimi-code/ })).toBeVisible();
  await page.getByTestId("workbench-project-trigger").click();
  const projectPopover = page.getByTestId("workbench-project-popover");
  await expect(projectPopover).toBeVisible();
  await expect(projectPopover).toHaveCSS("width", "320px");
  await expect(projectPopover.locator(".composer-popover-row-copy strong").first()).toHaveCSS(
    "font-size",
    "13px",
  );
  await expect(projectPopover.locator(".composer-popover-row-copy small").first()).toHaveCSS(
    "font-size",
    "12px",
  );
  if (process.env.HACHIMI_CAPTURE_COMPOSER_QA) {
    await page.screenshot({
      path: resolve(import.meta.dirname, "../../../../target/composer-project-popover-qa.png"),
      animations: "disabled",
    });
  }
  await page.locator(".welcome-block h1").click();
  await expect(page.getByTestId("workbench-project-popover")).toHaveCount(0);
  await page.getByTestId("workbench-execution-target").click();
  const executionPopover = page.getByTestId("workbench-execution-popover");
  await expect(executionPopover).toBeVisible();
  await expect(executionPopover).toHaveCSS("width", "292px");
  if (process.env.HACHIMI_CAPTURE_COMPOSER_QA) {
    await page.screenshot({
      path: resolve(import.meta.dirname, "../../../../target/composer-execution-popover-qa.png"),
      animations: "disabled",
    });
  }
  await page.getByTestId("workbench-execution-worktree").click();
  await page.getByTestId("workbench-base-branch").click();
  const branchPopover = page.getByTestId("workbench-branch-popover");
  await expect(branchPopover).toBeVisible();
  await expect(branchPopover).toHaveCSS("width", "350px");
  if (process.env.HACHIMI_CAPTURE_COMPOSER_QA) {
    await page.screenshot({
      path: resolve(import.meta.dirname, "../../../../target/composer-branch-popover-qa.png"),
      animations: "disabled",
    });
  }
  await page.locator(".welcome-block h1").click();
  await page.getByTestId("workbench-task-options").click();
  const optionsPopover = page.getByTestId("workbench-options-popover");
  await expect(optionsPopover).toBeVisible();
  await expect(optionsPopover).toHaveCSS("width", "390px");
  await expect(optionsPopover).toContainText("文件和文件夹");
  await expect(page.getByTestId("workbench-add-folder")).toHaveCount(0);
  await page.getByTestId("workbench-add-attachment").click();
  await expect(page.locator(".composer-attachment-card")).toHaveCount(1);
  await expect(page.locator(".composer-attachment-card")).toContainText("notes.txt");
  await page.locator(".composer-attachment-card").click();
  await expect(page.locator(".workbench-inspector")).toBeVisible();
  await expect(page.locator(".attachment-inspector")).toContainText("reference notes");
  await page.getByTestId("workbench-task-options").click();
  await page.getByTestId("workbench-skill-Documents").click();
  await expect(page.locator(".composer-skill-reference")).toContainText("Documents");
  if (process.env.HACHIMI_CAPTURE_COMPOSER_QA) {
    await page.screenshot({
      path: resolve(import.meta.dirname, "../../../../target/composer-options-popover-qa.png"),
      animations: "disabled",
    });
  }
  await page.locator(".welcome-block h1").click();
  await expect(page.getByTestId("workbench-options-popover")).toHaveCount(0);
  if (process.env.HACHIMI_CAPTURE_COMPOSER_QA) {
    await page.screenshot({
      path: resolve(import.meta.dirname, "../../../../target/composer-skill-reference-qa.png"),
      animations: "disabled",
    });
  }
  await page.getByTestId("workbench-permission-profile").click();
  const permissionPopover = page.getByTestId("workbench-permission-popover");
  await expect(permissionPopover).toBeVisible();
  await assertPermissionTones(page, permissionPopover);
  if (process.env.HACHIMI_CAPTURE_COMPOSER_QA) {
    await page.screenshot({
      path: resolve(import.meta.dirname, "../../../../target/composer-approval-popover-qa.png"),
      animations: "disabled",
    });
  }
  await page.locator("html").evaluate((root) => {
    root.style.setProperty("--font-size-xs", "16px");
    root.style.setProperty("--font-size-sm", "17px");
    root.style.setProperty("--font-size-md", "18px");
  });
  await expect(permissionPopover).toHaveCSS("width", "380px");
  await expect(permissionPopover.locator(".composer-popover-row-copy strong").first()).toHaveCSS(
    "font-size",
    "17px",
  );
  const enlargedPermissionBox = await permissionPopover.boundingBox();
  expect(enlargedPermissionBox).not.toBeNull();
  expect(enlargedPermissionBox!.height).toBeLessThan(240);
  if (process.env.HACHIMI_CAPTURE_COMPOSER_QA) {
    await page.screenshot({
      path: resolve(
        import.meta.dirname,
        "../../../../target/composer-approval-popover-large-type-qa.png",
      ),
      animations: "disabled",
    });
  }
});
for (const viewport of [
  { name: "1855x1343", width: 1855, height: 1343 },
  { name: "1440x900", width: 1440, height: 900 },
  { name: "1280x800", width: 1280, height: 800 },
  { name: "1024x768", width: 1024, height: 768 },
  { name: "960x640", width: 960, height: 640 },
  { name: "720x640", width: 720, height: 640 },
] as const) {
  test(`production workbench ${viewport.name}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await installTauriMocks(page);
    await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
    await expect(page.getByRole("heading", { name: /hachimi-code/ })).toBeVisible();
    await expect(page.getByTestId("workbench-open-location")).toHaveCount(0);
    await expect(page.getByTestId("workbench-pin-summary")).toHaveCount(0);
    await expect(page.locator('[data-component="composer"]')).toHaveCSS(
      "background-color",
      "rgb(32, 36, 42)",
    );
    const composerHeight = await page
      .locator('[data-component="composer"]')
      .evaluate((element) => element.getBoundingClientRect().height);
    expect(composerHeight).toBeLessThanOrEqual(120);
    await expect(page.locator('[data-component="title-bar"]')).toHaveCSS("height", "46px");
    if (viewport.width > 760) {
      await expect(page.locator(".project-sidebar")).toHaveCSS(
        "width",
        viewport.width <= 1100 ? "250px" : "288px",
      );
    } else {
      await expect(page.locator(".project-sidebar")).toBeHidden();
    }
    await expect(page.locator("html")).toHaveJSProperty("scrollWidth", viewport.width);
    await expect(page).toHaveScreenshot(`production-home-${viewport.name}.png`, {
      animations: "disabled",
    });
  });
}
for (const systemScheme of ["light", "dark"] as const) {
  test(`production system appearance ${systemScheme}`, async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 768 });
    await page.emulateMedia({ colorScheme: systemScheme });
    await installTauriMocks(page, false, false, false, "system");
    await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
    await expect(page.locator("html")).toHaveAttribute("data-color-scheme", systemScheme);
    await expect(page.locator('[data-component="title-bar"]')).toBeVisible();
    await expect(page).toHaveScreenshot(`production-home-system-${systemScheme}-1024x768.png`, {
      animations: "disabled",
    });
  });
}
test("production title bar and project rows preserve their desktop alignment and actions", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await installTauriMocks(page, true, false, true);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
  const titleMenus = page.locator(".title-menus");
  await expect(titleMenus.getByRole("button")).toHaveCount(4);
  await expect(titleMenus.getByRole("button").first()).toHaveCSS("white-space", "nowrap");
  await expect(page.locator(".window-controls").getByRole("button")).toHaveCount(3);
  await expect(page.locator(".window-controls").getByRole("button").last()).toBeVisible();
  const titleBounds = await page.locator('[data-component="title-bar"]').evaluate((element) => {
    const bounds = (selector: string) => {
      const rect = element.querySelector(selector)!.getBoundingClientRect();
      return { left: rect.left, right: rect.right, width: rect.width };
    };
    const rect = element.getBoundingClientRect();
    return {
      display: getComputedStyle(element).display,
      right: rect.right,
      history: bounds(".title-history"),
      menus: bounds(".title-menus"),
      controls: bounds(".window-controls"),
    };
  });
  expect(titleBounds.display).toBe("flex");
  expect(titleBounds.menus.left).toBeGreaterThanOrEqual(titleBounds.history.right);
  expect(titleBounds.controls.left).toBeGreaterThanOrEqual(titleBounds.menus.right);
  expect(titleBounds.controls.right).toBeLessThanOrEqual(titleBounds.right);
  const projectNames = page.locator(".project-row-name");
  await expect(projectNames).toHaveCount(2);
  const nameBoxes = await projectNames.evaluateAll((elements) =>
    elements.map((element) => ({
      x: element.getBoundingClientRect().x,
      text: element.textContent,
    })),
  );
  expect(nameBoxes.map(({ text }) => text)).toEqual(["notes", "hachimi-code"]);
  expect(Math.abs(nameBoxes[0]!.x - nameBoxes[1]!.x)).toBeLessThan(1);
  const hachimiRow = page.locator(".project-row-shell", { hasText: "hachimi-code" });
  await hachimiRow.hover();
  await expect(page.getByTestId("project-new-task-project-hachimi")).toBeVisible();
  await page.getByTestId("project-new-task-project-hachimi").click();
  await expect(page.getByRole("heading", { name: /hachimi-code/ })).toBeVisible();
  await expect(page.getByTestId("workbench-project-trigger")).toContainText("hachimi-code");
  const composerBox = await page.locator('[data-component="composer"]').boundingBox();
  const contextBox = await page.locator(".composer-context").boundingBox();
  const projectTriggerBox = await page.getByTestId("workbench-project-trigger").boundingBox();
  const composerInputBox = await page.locator('[data-component="composer-input"]').boundingBox();
  expect(composerBox).not.toBeNull();
  expect(contextBox).not.toBeNull();
  expect(projectTriggerBox).not.toBeNull();
  expect(composerInputBox).not.toBeNull();
  expect(Math.abs(contextBox!.x - composerBox!.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(projectTriggerBox!.x - composerInputBox!.x)).toBeLessThanOrEqual(1);

  await hachimiRow.hover();
  await page.getByTestId("project-more-project-hachimi").click();
  await expect(page.locator('[data-component="dropdown-item"]')).toHaveText([
    "置顶项目",
    "在资源管理器中打开",
    "创建永久工作树",
    "重命名项目",
    "全部标为已读",
    "归档任务",
    "移除",
  ]);
});

test("production density changes the shared root contract", async ({ page }) => {
  await installTauriMocks(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/appearance");
  const densityRow = page.locator('[data-component="settings-row"]', { hasText: "界面密度" });
  const density = densityRow.locator('[data-component="segmented-control"]');
  for (const option of [
    ["紧凑", "compact"],
    ["默认", "default"],
    ["宽松", "comfortable"],
  ] as const) {
    await density.getByRole("button", { name: option[0], exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("data-appearance-density", option[1]);
  }
});

test("project tools open before the first message and preserve the responsive inspector", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.clock.setFixedTime(new Date("2026-07-26T15:00:00.000Z"));
  await installTauriMocks(page, true, false, false);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
  await page.getByTestId("workbench-toggle-inspector").click();
  let menu = page.getByTestId("workbench-resource-menu");
  await expect(menu).toBeVisible();
  for (const label of ["审阅", "终端", "浏览器", "文件"]) {
    await expect(menu.getByRole("button", { name: label })).toBeEnabled();
  }
  await expect(page).toHaveScreenshot("production-project-tools-menu-1440x900.png", {
    animations: "disabled",
  });
  await menu.getByRole("button", { name: "审阅" }).click();
  await expect(page.locator('[data-component="workspace"][data-mode="review"]')).toBeVisible();
  await expect(page.locator('[data-component="diff"]')).toBeVisible();
  await expect(page.locator(".workspace-diff-file-list")).toContainText("home.tsx");
  await expect(page).toHaveScreenshot("production-project-review-1440x900.png", {
    animations: "disabled",
  });
  await page.getByTestId("workbench-toggle-inspector").click();
  await page.getByTestId("workbench-toggle-inspector").click();
  await expect(page.locator('[data-component="workspace"][data-mode="review"]')).toBeVisible();
  await page.getByTestId("workbench-inspector-new-tab").click();
  menu = page.getByTestId("workbench-resource-menu");
  await menu.getByRole("button", { name: "终端" }).click();
  await expect(
    page.locator(".workbench-bottom-panel .terminal-session.active .xterm"),
  ).toBeVisible();
  await menu.getByRole("button", { name: "文件" }).click();
  await expect(page.locator(".workspace-files-layout")).toBeVisible();
  await page.getByRole("button", { name: "README.md" }).click();
  await expect(page.locator(".workspace-file-tab-title")).toContainText("README.md");
  await expect(page.locator(".workspace-path-bar")).toContainText("README.md");
  await expect(page.locator(".composer-capability-note")).toContainText(
    "任务将使用全局模型设置的快照",
  );
  await expect(page.locator(".workbench-bottom-panel .terminal-panel")).toBeVisible();
  await expect(
    page.locator('.workbench-bottom-panel .terminal-tab[data-process-status="running"]'),
  ).toBeVisible();
  await expect(
    page.locator(".workbench-bottom-panel .terminal-session.active .xterm"),
  ).toBeVisible();
  await expect(page).toHaveScreenshot("production-project-tools-1440x900.png", {
    animations: "disabled",
  });

  await page.setViewportSize({ width: 960, height: 700 });
  await expect(page.locator(".workbench-inspector-wide")).toBeInViewport();
  await expect(page.locator("html")).toHaveJSProperty("scrollWidth", 960);
  await expect(page).toHaveScreenshot("production-project-tools-960x700.png", {
    animations: "disabled",
  });
});
test("production plan confirmation replaces the composer with a compact revision gate", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.clock.setFixedTime(new Date("2026-07-26T15:00:00.000Z"));
  await installTauriMocks(page, true, false, true, "dark", "plan");
  await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
  await page.getByTestId("project-select-project-hachimi").click();
  await page.getByTestId("session-select-session-ui-unification").click();

  await expect(page.getByTestId("workbench-execute-plan")).toBeVisible();
  await expect(page.getByTestId("workbench-revise-plan")).toBeVisible();
  await expect(page.locator('[data-component="composer"]')).toBeHidden();
  await expect(page.locator(".timeline-plan-card")).toContainText("Workbench Codex 对齐计划");
  await expect(page).toHaveScreenshot("production-plan-gate-1440x900.png", {
    animations: "disabled",
  });
  await page.setViewportSize({ width: 720, height: 640 });
  await expect(page.locator(".project-sidebar")).toBeHidden();
  await expect(page.locator("html")).toHaveJSProperty("scrollWidth", 720);
  expect(
    await page
      .locator(".workbench-gate")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  await expect(page).toHaveScreenshot("production-plan-gate-720x640.png");
});
test("production UserInput questions replace the composer with direct choices", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.clock.setFixedTime(new Date("2026-07-26T15:00:00.000Z"));
  await installTauriMocks(page, true, false, true, "dark", "user_input");
  await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
  await page.getByTestId("project-select-project-hachimi").click();
  await page.getByTestId("session-select-session-ui-unification").click();

  await expect(page.locator('[data-component="user-input-card"]')).toBeVisible();
  await expect(page.locator('[role="radiogroup"]')).toHaveCount(1);
  await expect(page.locator(".user-input-pager")).toContainText("1/2");
  await expect(page.getByTestId("workbench-submit-user-input")).toHaveCount(0);
  await expect(page.locator(".user-input-other-row input")).toBeVisible();
  await expect(page.locator('[data-component="composer"]')).toBeHidden();
  await expect(page).toHaveScreenshot("production-user-input-gate-1440x900.png", {
    animations: "disabled",
  });
  await page.setViewportSize({ width: 720, height: 640 });
  await expect(page.locator(".project-sidebar")).toBeHidden();
  await expect(page.locator("html")).toHaveJSProperty("scrollWidth", 720);
  expect(
    await page
      .locator(".workbench-gate")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  await expect(page).toHaveScreenshot("production-user-input-gate-720x640.png");
});

for (const route of ["general", "llm", "voice", "avatar", "skills", "mcp"] as const) {
  test(`production settings ${route} uses the shared page contract`, async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await installTauriMocks(page);
    await page.goto(`http://127.0.0.1:1420/workbench.html?route=settings/${route}`);
    await expect(page.getByRole("heading", { level: 1 }).first()).toBeVisible();
    await expect(page.locator("html")).toHaveJSProperty("scrollWidth", 1280);
    const result = await new AxeBuilder({ page })
      .include(".settings-main")
      .withTags(["wcag2a", "wcag2aa"])
      .disableRules(["nested-interactive"])
      .analyze();
    expect(result.violations).toEqual([]);
    if (route === "avatar") await expect(page.getByText("预览不可用")).toBeVisible();
    await expect(page).toHaveScreenshot(`production-settings-${route}-1280x800.png`, {
      animations: "disabled",
    });
  });
}
for (const viewport of [
  { name: "1855x1343", width: 1855, height: 1343 },
  { name: "1024x768", width: 1024, height: 768 },
  { name: "960x640", width: 960, height: 640 },
  { name: "720x640", width: 720, height: 640 },
] as const) {
  test(`production appearance ${viewport.name}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await installTauriMocks(page);
    await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/appearance");
    await expect(page.getByRole("heading", { name: "外观", exact: true })).toBeVisible();
    if (viewport.width > 760) {
      await expect(page.locator(".settings-sidebar")).toHaveCSS(
        "width",
        viewport.width <= 1040 ? "226px" : "256px",
      );
    } else {
      await expect(page.locator(".settings-sidebar")).toBeHidden();
    }
    const accentChip = page.locator(".accent-chip").first();
    await expect(accentChip).toHaveCSS("height", "24px");
    await expect(accentChip).toHaveCSS("background-color", "rgb(112, 98, 213)");
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
  await page.getByTestId("workbench-open-settings").click();
  await page.getByRole("button", { name: "配置" }).click();
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

  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/integrations");
  await expect(page.locator(".settings-page-heading h1")).toHaveText("平台集成");
  for (const provider of ["钉钉", "飞书", "企微 AI Bot", "企微自建应用", "微信 iLink / ClawBot"]) {
    await expect(page.getByRole("tab", { name: provider })).toBeVisible();
  }
  await page.getByRole("tab", { name: "企微自建应用" }).click();
  await expect(page.getByText("客户支持", { exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "钉钉" }).click();
  await expect(page.locator(".settings-nav button")).toHaveCount(14);
  for (const entry of [
    "通用",
    "外观",
    "语音",
    "配置",
    "宠物",
    "交互",
    "平台集成",
    "浏览器",
    "Computer Use",
    "Skills",
    "MCP",
    "Plugins",
    "Runtime & Security",
    "诊断",
  ]) {
    await expect(page.locator(".settings-nav").getByRole("button", { name: entry })).toBeVisible();
  }
  await expect(page.getByTestId("settings-nav-plugins")).toBeDisabled();
  await assertPlatformIntegrationsVisualMatrix(page);
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
    (element) => getComputedStyle(element).backgroundColor,
  );
  await sidebarSwitches.nth(1).click();
  await expect(root).toHaveAttribute("data-translucent-sidebar", "off");
  await expect
    .poll(() => sidebar.evaluate((element) => getComputedStyle(element).backgroundColor))
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
  await page.keyboard.press("Escape");
  const darkEditor = page.locator(".theme-profile-card").nth(1);
  await darkEditor.locator('[data-component="select-trigger"]').nth(0).click();
  await expect(page.getByRole("option")).toHaveCount(9);
  await page.keyboard.press("Escape");
  await lightEditor.locator('[data-component="select-trigger"]').nth(0).click();
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
    "Quiet Graphite",
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
  await expect(darkAccent).toHaveValue("#7062D5");

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
  await page
    .locator('[data-component="form-label"]', { hasText: "绑定动作" })
    .locator("..")
    .locator('[data-component="select-trigger"]')
    .click();
  await expect(page.getByRole("option", { name: /点头同意/ })).toBeVisible();
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

  const motionSearchBox = await page
    .locator(".motion-filter-control.search [data-component=search-field]")
    .boundingBox();
  const motionCategoryBox = await page
    .locator(".motion-filter-control.category [data-component=select-trigger]")
    .boundingBox();
  expect(motionSearchBox).not.toBeNull();
  expect(motionCategoryBox).not.toBeNull();
  expect(Math.abs(motionSearchBox!.y - motionCategoryBox!.y)).toBeLessThanOrEqual(1);
  expect(Math.abs(motionSearchBox!.height - motionCategoryBox!.height)).toBeLessThanOrEqual(1);

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
  expect(browserBox!.y).toBeLessThan(previewBox!.y);
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
  await expect(page.getByRole("button", { name: /VRMA 动作/ })).toHaveAttribute(
    "data-value",
    motionEntry.id,
  );
  await page.getByRole("button", { name: "暂停" }).click();
  const motionTimeSlider = page
    .locator('[data-component="range-field"]', { hasText: "动作时间" })
    .locator('input[type="range"]');
  await motionTimeSlider.fill("1100");
  await expect(motionTimeSlider).toHaveValue("1100");
  await expect(
    page.locator('[data-component="metric-card"]', { hasText: "活动骨骼" }).locator("strong"),
  ).not.toHaveText("0");
  await expect(page.getByText("30", { exact: true }).first()).toBeVisible();
  await expect(page.getByText("Root 轨迹", { exact: true })).toBeVisible();
  await expect(page.getByText("足部接触", { exact: true })).toBeVisible();
  await expect(page.getByText("接触时间线", { exact: true })).toBeVisible();
  await expect(page.getByText(/° · .* m/)).toBeVisible();
  const playbackSpeedSlider = page
    .locator('[data-component="range-field"]', { hasText: "播放速度" })
    .locator('input[type="range"]');
  await playbackSpeedSlider.fill("1.5");
  await expect(playbackSpeedSlider).toHaveValue("1.5");
  await expect(page).toHaveScreenshot("production-motion-lab-1280x800.png", {
    animations: "disabled",
    mask: [
      page.locator(".motion-lab-stage canvas"),
      page.locator('[data-component="metric-card"]', { hasText: "Solve" }).locator("strong"),
    ],
    maskColor: "#313236",
  });
});
test("resetting all local data requires explicit confirmation", async ({ page }) => {
  await installTauriMocks(page);
  await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/diagnostics");
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
      .disableRules(["aria-hidden-focus", "nested-interactive"])
      .analyze();
    expect(result.violations).toEqual([]);
  }
});
