import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { isAbsolute, join } from "node:path";
import { restartApplication, switchToWorkbench } from "../support/windows.mjs";

/* global DataTransfer, HTMLButtonElement, HTMLInputElement, HTMLTextAreaElement, InputEvent, document */

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

async function clickWhenReady(selector) {
  let clickedElement;
  await browser.waitUntil(
    async () => {
      try {
        const element = await $(selector);
        if (!(await element.isExisting())) return false;
        if (!(await element.isDisplayed())) return false;
        if (!(await element.isEnabled())) return false;
        await element.click();
        clickedElement = element;
        return true;
      } catch (error) {
        if (String(error).toLowerCase().includes("stale element reference")) return false;
        throw error;
      }
    },
    { timeout: 20_000, interval: 100, timeoutMsg: `${selector} was not ready to click` },
  );
  return clickedElement;
}

async function clickWorkspaceElement(selector) {
  const element = await $(selector);
  await element.waitForDisplayed({ timeout: 20_000 });
  await element.waitForEnabled({ timeout: 20_000 });
  await element.scrollIntoView({ block: "start", inline: "nearest" });
  await element.click();
  return element;
}

async function ensureDefaultMode() {
  const banner = await $(".plan-mode-banner");
  if (!(await banner.isExisting()) || !(await banner.isDisplayed())) return;
  await clickWhenReady('[data-testid="workbench-task-options"]');
  await clickWhenReady('[data-testid="workbench-plan-mode"]');
  await banner.waitForDisplayed({ reverse: true, timeout: 5_000 });
}

async function openProjectSessions() {
  const project = await $(".project-row");
  await project.waitForDisplayed({ timeout: 20_000 });
  if ((await project.getAttribute("aria-expanded")) !== "true") await project.click();
  await $(".project-sessions button").waitForDisplayed({ timeout: 20_000 });
}

async function selectWorkbenchOption(triggerSelector, label) {
  await clickWorkspaceElement(triggerSelector);
  const option = await $(
    `//*[contains(@data-component, "select-item") and contains(., "${label}")]`,
  );
  await option.waitForDisplayed({ timeout: 10_000 });
  await option.click();
}

async function submitEphemeralUserInput() {
  const input = await $('.user-input-question input[type="password"]');
  // A debug build starts a fresh checkout-bound Workspace Host and refreshes
  // AGENTS.md/readiness on both sides of the preceding write Tool boundary.
  await input.waitForDisplayed({ timeout: 75_000 });
  await input.setValue(EPHEMERAL_SECRET);
  await clickWhenReady('[data-testid="workbench-submit-user-input"]');
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
    await $('[data-testid="workbench-project-task-draft"]').waitForDisplayed({ timeout: 5_000 });
    await expect($('[data-testid="workbench-project-task-draft"]')).toHaveText(
      expect.stringContaining("新任务"),
    );
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
    const baseBranch = await $('[data-testid="workbench-base-branch"]');
    await baseBranch.waitForEnabled({ timeout: 20_000 });
    await browser.waitUntil(async () => (await baseBranch.getText()).includes("main"), {
      timeout: 20_000,
      timeoutMsg: "Managed Worktree base branch was not projected",
    });

    await clickWhenReady('[data-testid="workbench-task-options"]');
    await browser.execute(() => {
      const input = document.querySelector('[data-testid="workbench-attachment-file-input"]');
      if (!(input instanceof HTMLInputElement)) throw new Error("Attachment input is unavailable");
      const transfer = new DataTransfer();
      transfer.items.add(
        new File(["Use the deterministic Desktop E2E workflow.\n"], "reference.txt", {
          type: "text/plain",
        }),
      );
      Object.defineProperty(input, "files", { configurable: true, value: transfer.files });
      input.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await $(".composer-attachment-card").waitForDisplayed({ timeout: 10_000 });
    await clickWhenReady('[data-testid="workbench-plan-mode"]');
    await $(".plan-mode-banner").waitForDisplayed({ timeout: 5_000 });

    const draft = await $(".composer textarea");
    await draft.setValue("Create and verify the deterministic Desktop E2E evidence file.");
    await clickWhenReady('[data-testid="workbench-start-task"]');
    await $('[data-testid="workbench-execute-plan"]').waitForDisplayed({ timeout: 30_000 });
    await expect($(".workspace-diff-file")).not.toBeDisplayed();

    await clickWhenReady('[data-testid="workbench-execute-plan"]');
    await submitEphemeralUserInput();
    await clickWhenReady('[data-testid="workbench-approve-once"]');
    await browser.waitUntil(
      async () => (await $(".run-status-actions").getText()).includes("succeeded"),
      { timeout: 45_000, timeoutMsg: "Default Run did not succeed" },
    );
    await browser.waitUntil(
      async () => (await $(".workspace-file-tree").getText()).includes("desktop-e2e-evidence.txt"),
      { timeout: 20_000, timeoutMsg: "Workspace Watch did not project the new file" },
    );
    await $(".evidence-card").waitForDisplayed({ timeout: 20_000 });

    await clickWhenReady('[data-testid="workspace-diff-tab"]');
    const diff = await $(".workspace-diff-file");
    await diff.waitForDisplayed({ timeout: 20_000 });
    await expect(diff).toHaveText(expect.stringContaining("desktop-e2e-evidence.txt"));
    await diff.click();
    await expect($(".workspace-diff-hunk")).toHaveText(
      expect.stringContaining("Hachimi Desktop E2E evidence"),
    );

    await browser.refresh();
    await $(".session-timeline").waitForDisplayed({ timeout: 20_000 });
    await expect($(".run-status-actions")).toHaveText(expect.stringContaining("succeeded"));

    await restartApplication();
    await switchToWorkbench();
    await openProjectSessions();
    await $(".project-sessions button").click();
    await $(".evidence-card").waitForDisplayed({ timeout: 20_000 });
    await expect($(".run-status-actions")).toHaveText(expect.stringContaining("succeeded"));

    await clickWorkspaceElement('[data-testid="review-toggle"]');
    await clickWorkspaceElement('[data-testid="review-start"]');
    await browser.waitUntil(
      async () => {
        try {
          const finding = await $('[data-testid="review-finding"]');
          await finding.scrollIntoView();
          return (await finding.getText()).includes("desktop-e2e-evidence.txt:1");
        } catch {
          return false;
        }
      },
      { timeout: 45_000, timeoutMsg: "Inline Review finding was not projected" },
    );
    await browser.execute(() => {
      const button = document.querySelector('[data-testid^="review-finding-resolve-"]');
      if (!(button instanceof HTMLButtonElement)) throw new Error("Review resolve action missing");
      button.click();
    });
    await browser.waitUntil(
      async () => {
        try {
          return (await $('[data-testid="review-finding"]').getText()).includes("resolved");
        } catch {
          return false;
        }
      },
      { timeout: 10_000, timeoutMsg: "Review finding status was not persisted" },
    );

    await selectWorkbenchOption('[data-testid="review-delivery"]', "独立 Session");
    await clickWorkspaceElement('[data-testid="review-start"]');
    await browser.waitUntil(
      async () => (await $(".session-timeline-header h1").getText()).startsWith("Review:"),
      { timeout: 20_000, timeoutMsg: "Detached Review Session lineage was not opened" },
    );
    await clickWorkspaceElement('[data-testid="review-toggle"]');
    await browser.waitUntil(
      async () => {
        try {
          const finding = await $('[data-testid="review-finding"]');
          await finding.scrollIntoView();
          return (await finding.getText()).includes("Deterministic review finding");
        } catch {
          return false;
        }
      },
      { timeout: 45_000, timeoutMsg: "Detached Review finding was not projected" },
    );
    await expect($(".run-status-actions")).toHaveText(expect.stringContaining("succeeded"));

    await clickWorkspaceElement('[data-testid="workspace-files-tab"]');
    const evidenceFile = await $(
      '//*[contains(@class, "workspace-tree-entry") and contains(., "desktop-e2e-evidence.txt")]',
    );
    await evidenceFile.waitForDisplayed({ timeout: 20_000 });
    await evidenceFile.click();
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

    await clickWorkspaceElement('[data-testid="workspace-git-tab"]');
    let gitPanelState = "Git panel was not mounted";
    try {
      await browser.waitUntil(
        async () => {
          const panels = await $$(".workspace-git-panel");
          const visible = [];
          for (const panel of panels) {
            if (await panel.isDisplayed()) visible.push(await panel.getText());
          }
          gitPanelState = visible.length > 0 ? visible.join("\n---\n") : gitPanelState;
          return gitPanelState.includes("desktop-e2e-evidence.txt");
        },
        { timeout: 20_000 },
      );
    } catch (error) {
      throw new Error(`Git status did not include the edited evidence file:\n${gitPanelState}`, {
        cause: error,
      });
    }
    await clickWorkspaceElement('[data-testid="workspace-git-stage-all"]');
    const commitMessage = await $('[data-testid="workspace-git-commit-message"]');
    await commitMessage.setValue("Desktop E2E local commit");
    await clickWorkspaceElement('[data-testid="workspace-git-commit"]');
    await browser.waitUntil(
      async () =>
        (await $(".workspace-git-history").getText()).includes("Desktop E2E local commit"),
      { timeout: 30_000, timeoutMsg: "Local Git commit was not projected into history" },
    );
    assertSecretAbsent(process.env.HACHIMI_DATA_DIR, EPHEMERAL_SECRET);
  });

  it("recovers a Run interrupted while waiting for approval", async () => {
    await clickWhenReady('[data-testid="workbench-new-task"]');
    await ensureDefaultMode();
    const draft = await $(".composer textarea");
    await draft.setValue("Start the deterministic write and stop at approval.");
    await clickWhenReady('[data-testid="workbench-start-task"]');
    await submitEphemeralUserInput();
    await $('[data-testid="workbench-approve-once"]').waitForDisplayed({ timeout: 30_000 });

    const terminalOpen = await $(
      '//*[contains(@class, "terminal-panel")]//button[normalize-space(.)="打开" or normalize-space(.)="Open"]',
    );
    await terminalOpen.waitForEnabled({ timeout: 20_000 });
    await terminalOpen.click();
    const terminalInput = await $(
      '.terminal-panel input[placeholder*="输入命令"], .terminal-panel input[placeholder*="Type a command"]',
    );
    await terminalInput.waitForEnabled({ timeout: 20_000 });
    await terminalInput.setValue("Write-Output terminal-e2e");
    await clickWhenReady('.terminal-panel button[type="submit"]');
    await browser.waitUntil(
      async () => (await $(".terminal-output").getText()).includes("terminal-e2e"),
      { timeout: 20_000, timeoutMsg: "ConPTY stdin/output did not round-trip" },
    );
    const terminalPanel = await $(".terminal-panel");
    const sizeBefore = `${await terminalPanel.getAttribute("data-terminal-rows")}:${await terminalPanel.getAttribute("data-terminal-cols")}`;
    await browser.setWindowSize(1040, 700);
    await browser.waitUntil(
      async () => {
        const panel = await $(".terminal-panel");
        const current = `${await panel.getAttribute("data-terminal-rows")}:${await panel.getAttribute("data-terminal-cols")}`;
        return !current.includes("null") && current !== sizeBefore;
      },
      { timeout: 20_000, timeoutMsg: "Terminal resize was not acknowledged by ConPTY" },
    );
    await browser.setWindowSize(1280, 800);

    const project = process.env.HACHIMI_DESKTOP_E2E_PROJECT_PATH;
    if (!project) throw new Error("HACHIMI_DESKTOP_E2E_PROJECT_PATH is missing");
    const childStarted = join(project, "terminal-child-started.txt");
    const childSurvived = join(project, "terminal-grandchild-survived.txt");
    const childScript = `Start-Sleep -Seconds 4; Set-Content -LiteralPath '${childSurvived.replaceAll("'", "''")}' -Value escaped`;
    const encodedChild = Buffer.from(childScript, "utf16le").toString("base64");
    await terminalInput.setValue(
      `Set-Content -LiteralPath '${childStarted.replaceAll("'", "''")}' -Value started; Start-Process powershell.exe -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','${encodedChild}'`,
    );
    await clickWhenReady('.terminal-panel button[type="submit"]');
    await browser.waitUntil(() => existsSync(childStarted), {
      timeout: 10_000,
      timeoutMsg: "Terminal grandchild fixture did not start",
    });
    const terminalStop = await $(
      '//*[contains(@class, "terminal-panel")]//button[contains(., "终止") or contains(., "Stop")]',
    );
    await terminalStop.waitForEnabled({ timeout: 20_000 });
    await terminalStop.click();
    await browser.pause(5_000);
    expect(existsSync(childSurvived)).toBe(false);

    await clickWhenReady(
      '//*[contains(@class, "terminal-panel")]//button[normalize-space(.)="打开" or normalize-space(.)="Open"]',
    );
    const reconnectInput = await $(
      '.terminal-panel input[placeholder*="输入命令"], .terminal-panel input[placeholder*="Type a command"]',
    );
    await reconnectInput.setValue("Start-Sleep -Seconds 30");
    await clickWhenReady('.terminal-panel button[type="submit"]');
    await browser.refresh();
    await $(".terminal-panel").waitForDisplayed({ timeout: 20_000 });
    await browser.waitUntil(async () => (await $(".terminal-status").getText()) === "running", {
      timeout: 20_000,
      timeoutMsg: "Terminal did not reconnect after WebView reload",
    });
    const reconnectedStop = await $(
      '//*[contains(@class, "terminal-panel")]//button[contains(., "终止") or contains(., "Stop")]',
    );
    await reconnectedStop.click();

    await restartApplication();
    await switchToWorkbench();
    await openProjectSessions();
    const latestSession = await $(".project-sessions button");
    await latestSession.click();
    await expect($(".run-status-actions")).toHaveText(expect.stringContaining("interrupted"));
    await expect($('[data-testid="workbench-approve-once"]')).not.toBeDisplayed();
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
