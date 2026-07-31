import { clickWhenReady, isDisplayed, waitForDisplayed } from "../support/interactions.mjs";
import { switchToWorkbench } from "../support/windows.mjs";

/* global document */

async function waitForRun(status, timeout = 60_000) {
  await browser.waitUntil(async () => (await $(".run-status-actions").getText()).includes(status), {
    timeout,
    timeoutMsg: `Agent Run did not reach ${status}`,
  });
}

async function timelineText() {
  return browser.execute(() => document.querySelector(".timeline-items")?.textContent ?? "");
}

async function waitForTimeline(expected, timeout = 60_000) {
  await browser.waitUntil(async () => (await timelineText()).includes(expected), {
    timeout,
    timeoutMsg: `timeline did not include: ${expected}`,
  });
}

async function ensureProjectVisible() {
  await switchToWorkbench();
  if (!(await isDisplayed(".project-row"))) {
    await clickWhenReady('[data-testid="workbench-add-project"]');
    await waitForDisplayed(".project-row", 20_000);
  }
  const expanded = await browser.execute(
    () => document.querySelector(".project-row")?.getAttribute("aria-expanded") === "true",
  );
  if (!expanded) await clickWhenReady(".project-row");
}

describe("Hachimi Agent feature-flag ToolPlan fencing", () => {
  it("removes disabled tools from General and Coding model ToolPlans", async () => {
    await switchToWorkbench();
    await clickWhenReady('[data-testid="workbench-new-task"]');
    await $('[data-testid="workbench-composer-input"]').setValue(
      "[desktop-e2e:agent-feature-flags-disabled-general] inspect disabled tools",
    );
    await clickWhenReady('[data-testid="workbench-start-task"]');
    await waitForTimeline("General feature flags removed", 60_000);
    await waitForRun("succeeded");

    await ensureProjectVisible();
    await clickWhenReady('[data-testid^="project-new-task-"]');
    await waitForDisplayed('[data-testid="workbench-project-task-draft"]');
    await $('[data-testid="workbench-composer-input"]').setValue(
      "[desktop-e2e:agent-feature-flags-disabled-coding] inspect disabled tools",
    );
    await clickWhenReady('[data-testid="workbench-start-task"]');
    await waitForTimeline("Coding feature flags removed", 60_000);
    await waitForRun("succeeded");
  });
});
