import { clickWhenReady, isDisplayed, waitForDisplayed } from "../support/interactions.mjs";
import { restartApplication, switchToWorkbench } from "../support/windows.mjs";

/* global document */

async function waitForRun(status, timeout = 60_000) {
  await browser.waitUntil(
    async () =>
      (await $('[data-testid="workbench-session-timeline"]').getAttribute("data-run-status")) ===
      status,
    { timeout, timeoutMsg: `Agent Run did not reach ${status}` },
  );
}

async function ensureDefaultMode() {
  if (!(await isDisplayed('[data-testid="workbench-plan-mode-chip"]'))) return;
  await clickWhenReady('[data-testid="workbench-plan-mode-chip"]');
  await browser.waitUntil(
    async () => !(await isDisplayed('[data-testid="workbench-plan-mode-chip"]')),
    { timeout: 5_000, timeoutMsg: "Plan mode did not close" },
  );
}

async function ensurePlanMode() {
  if (await isDisplayed('[data-testid="workbench-plan-mode-chip"]')) return;
  await clickWhenReady('[data-testid="workbench-task-options"]');
  await clickWhenReady('[data-testid="workbench-plan-mode"]');
  await waitForDisplayed('[data-testid="workbench-plan-mode-chip"]');
}

async function ensureWritableProfile() {
  await clickWhenReady('[data-testid="workbench-permission-profile"]');
  await waitForDisplayed('[data-testid="workbench-permission-popover"]');
  await clickWhenReady('[data-testid="workbench-permission-writable"]');
  await browser.waitUntil(
    async () => !(await isDisplayed('[data-testid="workbench-permission-popover"]')),
    { timeout: 5_000, timeoutMsg: "Writable permission selection did not close" },
  );
}

async function expandProject() {
  if (!(await isDisplayed(".project-row"))) {
    await clickWhenReady('[data-testid="workbench-add-project"]');
  }
  await waitForDisplayed(".project-row");
  const expanded = await browser.execute(
    () => document.querySelector(".project-row")?.getAttribute("aria-expanded") === "true",
  );
  if (!expanded) await clickWhenReady(".project-row");
}

async function startTask(marker, instruction, options = {}) {
  await switchToWorkbench();
  await expandProject();
  await clickWhenReady('[data-testid^="project-new-task-"]');
  await waitForDisplayed('[data-testid="workbench-composer-input"]');
  if (options.plan) {
    await ensurePlanMode();
  } else {
    await ensureDefaultMode();
    await ensureWritableProfile();
  }
  await $('[data-testid="workbench-composer-input"]').setValue(`${marker} ${instruction}`);
  await clickWhenReady('[data-testid="workbench-start-task"]');
}

async function reopenProjectSession(sessionTestId) {
  await switchToWorkbench();
  await expandProject();
  const selector = sessionTestId ? `[data-testid="${sessionTestId}"]` : ".project-sessions button";
  await waitForDisplayed(selector);
  await clickWhenReady(selector);
  await waitForDisplayed('[data-testid="workbench-session-timeline"]');
}

async function restartAndReopen() {
  const selected = await $('[data-testid^="session-select-"][aria-current="page"]');
  const sessionTestId = (await selected.isExisting())
    ? await selected.getAttribute("data-testid")
    : undefined;
  await restartApplication();
  await reopenProjectSession(sessionTestId);
}

async function abandonRecovery(reasonCode) {
  await waitForRun("waiting_recovery_decision", 30_000);
  const recovery = await $('[data-testid^="run-recovery-"]');
  await recovery.waitForDisplayed({ timeout: 30_000 });
  await expect(recovery).toHaveText(expect.stringContaining(reasonCode));
  await clickWhenReady('[data-testid^="run-recovery-"] footer button');
  await waitForRun("cancelled", 20_000);
}

describe("Hachimi App Server resume/rejoin process matrix", () => {
  it("invalidates a waiting approval on restart", async () => {
    const fixtureUrl = process.env.HACHIMI_DESKTOP_E2E_BROWSER_URL;
    if (!fixtureUrl) throw new Error("HACHIMI_DESKTOP_E2E_BROWSER_URL is missing");
    await startTask(
      "[desktop-e2e:approval-recovery]",
      `url=${fixtureUrl} stop at the approval boundary.`,
    );
    await waitForDisplayed('[data-testid="workbench-approve-once"]', 30_000);
    await restartAndReopen();
    await expect($('[data-testid="workbench-approve-once"]')).not.toBeDisplayed();
    await abandonRecovery("approval_expired_on_restart");
  });

  it("invalidates waiting user input on restart", async () => {
    await startTask(
      "[desktop-e2e:user-input-recovery]",
      "stop while waiting for ephemeral user input.",
      { plan: true },
    );
    await waitForDisplayed(".user-input-card", 30_000);
    await restartAndReopen();
    await expect($(".user-input-card")).not.toBeDisplayed();
    await abandonRecovery("user_input_interrupted_on_restart");
  });

  it("auto-resumes one read-only checkpoint on the next generation", async () => {
    await startTask(
      "[desktop-e2e:read-only-recovery]",
      "read the fixture and block after the durable checkpoint.",
    );
    await waitForRun("running", 30_000);
    await browser.pause(1_000);
    await restartAndReopen();
    await waitForRun("succeeded", 45_000);
    await expect($(".timeline-assistant .timeline-message-text")).toHaveText(
      expect.stringContaining("read-only checkpoint resumed on generation 2"),
    );
  });

  it("reuses one reliable idempotent receipt after restart", async () => {
    await startTask(
      "[desktop-e2e:idempotent-receipt-recovery]",
      "spawn one bounded child and block after its durable receipt.",
    );
    await waitForRun("running", 45_000);
    await browser.pause(1_000);
    await restartAndReopen();
    await waitForRun("succeeded", 60_000);
    await expect($(".timeline-assistant .timeline-message-text")).toHaveText(
      expect.stringContaining("idempotent receipt resumed exactly once on generation 2"),
    );
  });

  it("marks dispatch without receipt indeterminate and never replays it", async () => {
    await startTask(
      "[desktop-e2e:indeterminate-recovery]",
      "dispatch the debug external effect and block before its receipt.",
    );
    await browser.pause(1_000);
    if (await isDisplayed('[data-testid="workbench-approve-once"]')) {
      await clickWhenReady('[data-testid="workbench-approve-once"]');
    }
    await waitForRun("running", 30_000);
    await browser.pause(1_000);
    await restartAndReopen();
    await abandonRecovery("side_effect_result_indeterminate");
  });
});
