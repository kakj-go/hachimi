import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { isAbsolute, join } from "node:path";
import {
  clickWhenReady,
  hoverWhenReady,
  isDisplayed,
  waitForDisplayed,
} from "../support/interactions.mjs";
import { restartApplication, switchToPet, switchToWorkbench } from "../support/windows.mjs";

/* global HTMLButtonElement, HTMLElement, HTMLTextAreaElement, InputEvent, document */

const EPHEMERAL_SECRET = "desktop-e2e-secret-value";

function git(project, ...args) {
  const result = spawnSync("git", args, { cwd: project, encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`git ${args.join(" ")} failed: ${result.stderr || result.stdout}`);
  }
  return result.stdout.trim();
}

function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

async function clickWorkspaceElement(selector) {
  await clickWhenReady(selector);
}

async function waitForRun(status, timeout = 60_000) {
  await browser.waitUntil(
    async () =>
      (await $('[data-testid="workbench-session-timeline"]').getAttribute("data-run-status")) ===
      status,
    {
      timeout,
      timeoutMsg: `Agent Run did not reach ${status}`,
    },
  );
}

async function ensureDefaultMode() {
  if (!(await isDisplayed('[data-testid="workbench-plan-mode-chip"]'))) return;
  await clickWhenReady('[data-testid="workbench-plan-mode-chip"]');
  await browser.waitUntil(
    async () => !(await isDisplayed('[data-testid="workbench-plan-mode-chip"]')),
    {
      timeout: 5_000,
      timeoutMsg: "Plan mode chip did not close",
    },
  );
}

async function expandFirstProject() {
  await waitForDisplayed(".project-row");
  const expanded = await browser.execute(
    () => document.querySelector(".project-row")?.getAttribute("aria-expanded") === "true",
  );
  if (!expanded) await clickWhenReady(".project-row");
}

async function openProjectSessions() {
  await expandFirstProject();
  await waitForDisplayed(".project-sessions button");
}

async function openInspectorToolLauncher() {
  if (await isDisplayed('[data-testid="workbench-resource-menu"]')) return;
  if (!(await isDisplayed(".workbench-inspector"))) {
    await clickWhenReady('[data-testid="workbench-toggle-inspector"]');
    await waitForDisplayed(".workbench-inspector");
  }
  await clickWhenReady('[data-testid="workbench-inspector-new-tab"]');
  await waitForDisplayed('[data-testid="workbench-resource-menu"]');
}

async function submitEphemeralUserInput() {
  // A debug build starts a fresh checkout-bound Workspace Host and refreshes
  // AGENTS.md/readiness on both sides of the preceding write Tool boundary.
  const selector = '.user-input-question input[type="password"]';
  await waitForDisplayed(selector, 75_000);
  await $(selector).setValue(EPHEMERAL_SECRET);
  await clickWhenReady('[data-testid="workbench-submit-user-input"]');
}

async function writeTerminal(command) {
  const terminal = await $(".terminal-session.active .xterm");
  await terminal.waitForDisplayed({ timeout: 20_000 });
  await terminal.click();
  await browser.keys(command);
  await browser.keys("Enter");
}

describe("Hachimi Workbench core lifecycle", () => {
  it("reconciles late Git init, creates a true empty root commit, and opens a draft lazily", async () => {
    await switchToWorkbench();
    await expect($(".sandbox-readiness-banner")).not.toBeDisplayed();
    await clickWhenReady('[data-testid="workbench-add-project"]');
    await browser.waitUntil(
      async () =>
        (await $('[data-testid="workbench-project-git-state"]').getText()).includes("非 Git"),
      { timeout: 20_000, timeoutMsg: "initial non-Git inspection did not settle" },
    );
    await expect($('[data-testid="workbench-project-git-state"]')).toHaveText(
      expect.stringContaining("非 Git"),
    );

    const project = process.env.HACHIMI_DESKTOP_E2E_PROJECT_PATH;
    if (!project) throw new Error("HACHIMI_DESKTOP_E2E_PROJECT_PATH is missing");
    git(project, "init", "--initial-branch=main");
    writeFileSync(join(project, "staged.txt"), "staged but not committed\n", "utf8");
    writeFileSync(join(project, "untracked.txt"), "untracked\n", "utf8");
    git(project, "add", "staged.txt");
    const indexPath = git(project, "rev-parse", "--git-path", "index");
    const absoluteIndex = isAbsolute(indexPath) ? indexPath : join(project, indexPath);
    const indexBefore = sha256File(absoluteIndex);

    await clickWhenReady('[aria-label="刷新 Git 状态"]');
    await browser.waitUntil(
      async () =>
        (await $('[data-testid="workbench-project-git-state"]').getText()).includes("尚无提交"),
      { timeout: 20_000, timeoutMsg: "late git init was not reconciled as an unborn branch" },
    );
    await expect($('[data-testid="workbench-project-git-state"]')).toHaveText(
      expect.stringContaining("main"),
    );

    await clickWhenReady('[data-testid="project-git-create-initial"]');
    const identity = await $$(".project-git-initial-fields input");
    await identity[0].setValue("Hachimi Desktop E2E");
    await identity[1].setValue("desktop-e2e@hachimi.invalid");
    await clickWhenReady('[data-testid="project-git-create-initial-confirm"]');
    await browser.waitUntil(
      async () =>
        !(await $('[data-testid="workbench-project-git-state"]').getText()).includes("尚无提交"),
      { timeout: 20_000, timeoutMsg: "empty initial commit did not make the repository ready" },
    );

    expect(sha256File(absoluteIndex)).toBe(indexBefore);
    expect(git(project, "diff", "--cached", "--name-only")).toBe("staged.txt");
    expect(git(project, "ls-tree", "--name-only", "HEAD")).toBe("");

    git(project, "checkout", "--detach", "HEAD");
    await clickWhenReady('[aria-label="刷新 Git 状态"]');
    await browser.waitUntil(
      async () =>
        (await $('[data-testid="workbench-project-git-state"]').getText()).includes("detached"),
      { timeout: 20_000, timeoutMsg: "detached HEAD was not projected" },
    );
    await clickWhenReady('[data-testid="workbench-execution-target"]');
    await expect($('[data-testid="workbench-execution-worktree"]')).toBeDisabled();
    await clickWhenReady('[data-testid="workbench-execution-target"]');

    git(project, "switch", "main");
    await clickWhenReady('[aria-label="刷新 Git 状态"]');
    await browser.waitUntil(
      async () => (await $('[data-testid="workbench-project-git-state"]').getText()) === "main",
      { timeout: 20_000, timeoutMsg: "switching back to main was not reconciled" },
    );

    await clickWhenReady('[data-testid="workbench-execution-target"]');
    await $('[data-testid="workbench-execution-worktree"]').waitForEnabled({ timeout: 20_000 });
    await clickWhenReady('[data-testid="workbench-execution-worktree"]');
    await $('[data-testid="workbench-base-branch"]').waitForEnabled({ timeout: 20_000 });
    await browser.waitUntil(
      async () => (await $('[data-testid="workbench-base-branch"]').getText()).includes("main"),
      { timeout: 20_000, timeoutMsg: "initial commit refs did not select the current branch" },
    );
    await expect($('[data-testid="workbench-base-branch"]')).toHaveText(
      expect.stringContaining("main"),
    );

    await browser.execute(() => document.querySelector(".project-row")?.focus());
    await clickWhenReady(".project-new-task");
    await $('[data-testid="workbench-composer-input"]').waitForDisplayed({ timeout: 5_000 });
    await expect($('[data-testid="workbench-project-task-draft"]')).not.toExist();
    const composerFocused = await browser.execute(() => {
      const composer = document.querySelector('[data-testid="workbench-composer-input"]');
      return document.activeElement === composer;
    });
    expect(composerFocused).toBe(true);
    await expect($(".project-sessions button")).not.toExist();
  });

  it("plans read-only, accepts, approves, asks, verifies, diffs and resumes", async () => {
    await switchToWorkbench();
    await expect($(".sandbox-readiness-banner")).not.toBeDisplayed();
    await clickWhenReady('[data-testid="workbench-execution-target"]');
    await clickWhenReady('[data-testid="workbench-execution-worktree"]');
    await browser.waitUntil(
      async () =>
        browser.execute(() => {
          const target = document.querySelector('[data-testid="workbench-base-branch"]');
          return (
            target instanceof HTMLButtonElement &&
            !target.disabled &&
            (target.textContent?.includes("main") ?? false)
          );
        }),
      {
        timeout: 20_000,
        timeoutMsg: "Managed Worktree base branch was not projected",
      },
    );

    await clickWhenReady('[data-testid="workbench-task-options"]');
    await clickWhenReady('[data-testid="workbench-add-attachment"]');
    await $(".composer-attachment-card").waitForDisplayed({ timeout: 10_000 });
    await clickWhenReady('[data-testid="workbench-task-options"]');
    await clickWhenReady('[data-testid="workbench-plan-mode"]');
    await $('[data-testid="workbench-plan-mode-chip"]').waitForDisplayed({ timeout: 5_000 });

    const draft = await $(".composer textarea");
    await draft.setValue("Create and verify the deterministic Desktop E2E evidence file.");
    await clickWhenReady('[data-testid="workbench-start-task"]');
    await $('[data-testid="workbench-execute-plan"]').waitForDisplayed({ timeout: 30_000 });
    await expect($(".workspace-diff-file")).not.toBeDisplayed();

    await clickWhenReady('[data-testid="workbench-execute-plan"]');
    await submitEphemeralUserInput();
    await clickWhenReady('[data-testid="workbench-approve-once"]');
    await waitForRun("succeeded", 45_000);
    await clickWhenReady('[data-testid="workbench-pin-summary"]');
    await clickWhenReady('[data-testid="workbench-summary-files"]');
    await browser.waitUntil(
      async () => (await $(".workspace-file-tree").getText()).includes("desktop-e2e-evidence.txt"),
      { timeout: 20_000, timeoutMsg: "Workspace Watch did not project the new file" },
    );
    await expect($(".run-completion-summary")).toHaveText(
      expect.stringContaining("desktop-e2e-evidence.txt"),
    );

    await clickWhenReady('[data-testid="workbench-summary-diff"]');
    await waitForDisplayed(".workspace-diff-tree-entry[data-status]");
    await browser.waitUntil(
      async () =>
        browser.execute(
          () =>
            document
              .querySelector(".workspace-diff-file-list")
              ?.textContent?.includes("desktop-e2e-evidence.txt") ?? false,
        ),
      { timeout: 20_000, timeoutMsg: "Workspace diff file was not projected" },
    );
    await clickWhenReady(
      '//*[contains(@class, "workspace-diff-tree-entry") and contains(., "desktop-e2e-evidence.txt")]',
    );
    await expect($(".workspace-diff-hunk")).toHaveText(
      expect.stringContaining("Hachimi Desktop E2E evidence"),
    );

    await browser.refresh();
    await $(".session-timeline").waitForDisplayed({ timeout: 20_000 });
    await waitForRun("succeeded", 20_000);

    await restartApplication();
    await switchToWorkbench();
    await openProjectSessions();
    await clickWhenReady(".project-sessions button");
    await expect($(".run-completion-summary")).toHaveText(
      expect.stringContaining("desktop-e2e-evidence.txt"),
    );
    await waitForRun("succeeded", 20_000);

    if (!(await isDisplayed('[data-testid="workbench-summary-files"]'))) {
      await clickWorkspaceElement('[data-testid="workbench-pin-summary"]');
    }
    await clickWorkspaceElement('[data-testid="workbench-summary-files"]');
    const evidenceFileSelector =
      '//*[contains(@class, "workspace-tree-entry") and contains(., "desktop-e2e-evidence.txt")]';
    await waitForDisplayed(evidenceFileSelector);
    await clickWhenReady(evidenceFileSelector);
    await $(".workspace-monaco-editor.ready").waitForDisplayed({ timeout: 20_000 });
    await browser.execute(() => {
      const editor = document.querySelector('[data-testid="workspace-editor-fallback"]');
      if (!(editor instanceof HTMLTextAreaElement)) throw new Error("Workspace editor is missing");
      editor.value = `${editor.value.trimEnd()}\nEdited through the Workbench editor.\n`;
      editor.dispatchEvent(new InputEvent("input", { bubbles: true }));
    });
    await clickWorkspaceElement('[data-testid="workspace-save-file"]');
    await browser.waitUntil(
      async () => !(await $('[data-testid="workspace-save-file"]').isEnabled()),
      { timeout: 20_000, timeoutMsg: "Workspace editor save did not become authoritative" },
    );

    await expect($('[data-testid="workspace-git-tab"]')).not.toExist();
    await expect($('[data-testid="workspace-diff-tab"]')).not.toExist();
    assertSecretAbsent(process.env.HACHIMI_DATA_DIR, EPHEMERAL_SECRET);
  });

  it("recovers a Run interrupted while waiting for approval", async () => {
    await expandFirstProject();
    await clickWhenReady('[data-testid^="project-new-task-"]');
    await $('[data-testid="workbench-composer-input"]').waitForDisplayed({ timeout: 5_000 });
    await ensureDefaultMode();
    const draft = await $(".composer textarea");
    await draft.setValue("Start the deterministic write and stop at approval.");
    await clickWhenReady('[data-testid="workbench-start-task"]');
    await submitEphemeralUserInput();
    await $('[data-testid="workbench-approve-once"]').waitForDisplayed({ timeout: 30_000 });

    await openInspectorToolLauncher();
    await clickWhenReady(
      '//*[@data-testid="workbench-resource-menu"]//button[contains(., "终端") or contains(., "Terminal")]',
    );
    await waitForDisplayed(".workbench-bottom-panel .terminal-session.active .xterm");

    const project = process.env.HACHIMI_DESKTOP_E2E_PROJECT_PATH;
    if (!project) throw new Error("HACHIMI_DESKTOP_E2E_PROJECT_PATH is missing");
    await writeTerminal('Get-Location; Write-Output "terminal-e2e"');
    await browser.waitUntil(
      async () => {
        const text = await $(".terminal-session.active .xterm-rows").getText();
        return text.includes("terminal-e2e") && text.toLowerCase().includes(project.toLowerCase());
      },
      { timeout: 20_000, timeoutMsg: "Project terminal cwd or PTY output was incorrect" },
    );
    const processId = await $(".terminal-session.active").getAttribute("data-process-id");
    await clickWhenReady('[aria-label="隐藏终端面板"], [aria-label="Hide terminal panel"]');
    await expect($(".workbench-bottom-panel")).not.toExist();
    await clickWhenReady(
      '//*[@data-testid="workbench-resource-menu"]//button[contains(., "终端") or contains(., "Terminal")]',
    );
    await waitForDisplayed(".workbench-bottom-panel .terminal-session.active .xterm");
    expect(await $(".terminal-session.active").getAttribute("data-process-id")).toBe(processId);

    await browser.setWindowSize(1040, 700);
    await expect($(".terminal-session.active .xterm")).toBeDisplayed();
    await browser.setWindowSize(1280, 800);

    const childStarted = join(project, "terminal-child-started.txt");
    const childSurvived = join(project, "terminal-grandchild-survived.txt");
    const childScript = `Start-Sleep -Seconds 4; Set-Content -LiteralPath '${childSurvived.replaceAll("'", "''")}' -Value escaped`;
    const encodedChild = Buffer.from(childScript, "utf16le").toString("base64");
    await writeTerminal(
      `Set-Content -LiteralPath '${childStarted.replaceAll("'", "''")}' -Value started; Start-Process powershell.exe -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','${encodedChild}'`,
    );
    await browser.waitUntil(() => existsSync(childStarted), {
      timeout: 10_000,
      timeoutMsg: "Terminal grandchild fixture did not start",
    });
    await clickWhenReady('[aria-label="关闭终端"], [aria-label="Close terminal"]');
    await browser.pause(5_000);
    expect(existsSync(childSurvived)).toBe(false);

    await openInspectorToolLauncher();
    await clickWhenReady(
      '//*[@data-testid="workbench-resource-menu"]//button[contains(., "终端") or contains(., "Terminal")]',
    );
    await waitForDisplayed(".terminal-session.active .xterm");
    await writeTerminal("Start-Sleep -Seconds 30");
    await browser.refresh();
    await $(".terminal-panel").waitForDisplayed({ timeout: 20_000 });
    await browser.waitUntil(
      async () => (await $('.terminal-tab[data-process-status="running"]').isDisplayed()) === true,
      { timeout: 20_000, timeoutMsg: "Terminal did not reconnect after WebView reload" },
    );
    await clickWhenReady('[aria-label="关闭终端"], [aria-label="Close terminal"]');

    await restartApplication();
    await switchToWorkbench();
    await openProjectSessions();
    await clickWhenReady(".project-sessions button");
    await waitForRun("waiting_recovery_decision", 20_000);
    const recoveryCard = await $('[data-testid^="run-recovery-"]');
    await recoveryCard.waitForDisplayed({ timeout: 20_000 });
    await expect(recoveryCard).toHaveText(expect.stringContaining("approval_expired_on_restart"));
    await expect(recoveryCard).toHaveText(expect.stringContaining("generation 1 → 2"));
    await expect($('[data-testid="workbench-approve-once"]')).not.toBeDisplayed();
    await clickWhenReady('[data-testid^="run-recovery-"] footer button');
    await waitForRun("cancelled", 20_000);
    await expect($('[data-testid^="run-recovery-"]')).not.toBeDisplayed();
  });

  it("continues one Pet Run across Workbench UserInput and Pet Approval", async () => {
    const petSecret = "desktop-e2e-pet-cross-window-secret";
    await switchToPet();
    await hoverWhenReady(".pet-avatar-hit-area");
    await waitForDisplayed('[data-testid="pet-permission-toggle"]', 10_000);
    const permissionEnabled = await browser.execute(
      () =>
        document
          .querySelector('[data-testid="pet-permission-toggle"]')
          ?.getAttribute("aria-pressed") === "true",
    );
    if (!permissionEnabled) await clickWhenReady('[data-testid="pet-permission-toggle"]');
    await hoverWhenReady(".pet-avatar-hit-area");
    await clickWhenReady('[data-testid="pet-open-composer"]');
    await $('[data-testid="pet-composer-input"]').setValue(
      "[desktop-e2e:pet-cross-window] verify shared interaction ownership",
    );
    await clickWhenReady('[data-testid="pet-composer-submit"]');
    await waitForDisplayed('[data-testid="pet-attention"]', 60_000);
    await browser.waitUntil(
      async () =>
        browser.execute(
          () =>
            document
              .querySelector('[data-testid="pet-attention"]')
              ?.textContent?.includes("Enter the Pet cross-window") ?? false,
        ),
      { timeout: 60_000, timeoutMsg: "Pet UserInput prompt was not projected" },
    );
    const agentRunId = await browser.execute(() =>
      document.querySelector('[data-testid="pet-stage"]')?.getAttribute("data-agent-run-id"),
    );
    expect(agentRunId).toBeTruthy();

    await clickWhenReady('[data-testid="pet-open-workbench"]');
    await switchToWorkbench();
    await browser.refresh();
    await waitForDisplayed(".general-sessions");
    const petSessionSelector =
      '//div[contains(@class, "general-sessions")]//button[contains(., "[desktop-e2e:pet-cross-window]")]';
    await waitForDisplayed(petSessionSelector);
    await clickWhenReady(petSessionSelector);
    const workbenchInputSelector = '.user-input-question input[type="password"]';
    await waitForDisplayed(workbenchInputSelector, 30_000);
    await $(workbenchInputSelector).setValue(petSecret);
    await clickWhenReady('[data-testid="workbench-submit-user-input"]');
    await clickWhenReady(".window-close");

    await switchToPet();
    await waitForDisplayed('[data-testid="pet-approve-once"]', 30_000);
    expect(await $('[data-testid="pet-stage"]').getAttribute("data-agent-run-id")).toBe(agentRunId);
    await browser.execute(() => {
      const button = document.querySelector('[data-testid="pet-approve-once"]');
      if (!(button instanceof HTMLButtonElement)) throw new Error("Pet Approval action is missing");
      button.click();
    });
    await browser.waitUntil(
      async () => {
        const state = await browser.execute(() => {
          const approvalButton = document.querySelector('[data-testid="pet-approve-once"]');
          const error = document.querySelector('[data-testid="pet-attention-error"]');
          return {
            approvalVisible:
              approvalButton instanceof HTMLElement && approvalButton.offsetParent !== null,
            errorVisible: error instanceof HTMLElement && error.offsetParent !== null,
            errorText: error?.textContent ?? "",
          };
        });
        if (!state.approvalVisible) return true;
        if (state.errorVisible) throw new Error(`Pet Approval failed: ${state.errorText}`);
        return false;
      },
      { timeout: 20_000, timeoutMsg: "Pet Approval did not resolve" },
    );
    await browser.waitUntil(
      async () =>
        browser.execute(
          () =>
            document.querySelector(".pet-speech")?.textContent?.includes("one Agent Run") ?? false,
        ),
      { timeout: 45_000, timeoutMsg: "Pet completion reply was not projected" },
    );
    assertSecretAbsent(process.env.HACHIMI_DATA_DIR, petSecret);
  });
});

function assertSecretAbsent(root, secret) {
  const pending = [root];
  while (pending.length > 0) {
    const directory = pending.pop();
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) {
        if (path === join(root, "webview")) continue;
        pending.push(path);
      } else if (entry.isFile()) {
        const bytes = readFileSync(path);
        if (bytes.includes(Buffer.from(secret))) {
          throw new Error(`ephemeral UserInput secret was persisted in ${entry.name}`);
        }
      }
    }
  }
}
