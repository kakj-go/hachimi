import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { validateOfficeArtifact } from "../support/office-artifacts.mjs";
import { restartApplication, switchToWorkbench } from "../support/windows.mjs";
import { assertWindowsToast } from "../support/windows-toast.mjs";

const restrictedStdioOffice = process.env.HACHIMI_DESKTOP_E2E_REAL_SANDBOX === "1";

async function clickWhenReady(selector) {
  const element = await $(selector);
  await element.waitForDisplayed({ timeout: 20_000 });
  await element.waitForEnabled({ timeout: 20_000 });
  await element.click();
  return element;
}

async function setValueWhenReady(selector, value) {
  const element = await $(selector);
  await element.waitForDisplayed({ timeout: 20_000 });
  await element.waitForEnabled({ timeout: 20_000 });
  await element.waitForClickable({ timeout: 20_000 });
  await element.setValue(value);
  return element;
}

async function openTaskCenter() {
  await clickWhenReady('[data-testid="workbench-task-tab"]');
  await $('[data-testid="workbench-task-center"]').waitForDisplayed({ timeout: 20_000 });
}

async function ensureOfficeMcp() {
  const serverName = restrictedStdioOffice
    ? "Desktop E2E Restricted Office MCP"
    : "Desktop E2E MCP";
  await switchToWorkbench();
  if (!(await $(".settings-nav").isDisplayed())) {
    await clickWhenReady('[data-testid="workbench-open-settings"]');
    await $(".settings-nav").waitForDisplayed({ timeout: 20_000 });
  }
  await clickWhenReady('[data-testid="settings-nav-mcp"]');
  await $('[data-testid="mcp-settings-page"]').waitForDisplayed({ timeout: 20_000 });

  let existing;
  for (const row of await $$(".mcp-server-row")) {
    if ((await row.getText()).includes(serverName)) {
      existing = row;
      break;
    }
  }
  if (existing) {
    await existing.$(".mcp-server-select").click();
  } else {
    await clickWhenReady('[data-testid="mcp-add-server"]');
    await setValueWhenReady('[data-testid="mcp-create-name"]', serverName);
    if (restrictedStdioOffice) {
      const stdio = await $('//button[contains(., "Local stdio") or contains(., "本地 stdio")]');
      await stdio.waitForDisplayed({ timeout: 20_000 });
      await stdio.waitForClickable({ timeout: 20_000 });
      await stdio.click();
      await setValueWhenReady(
        '[data-testid="mcp-create-stdio-command"]',
        process.env.HACHIMI_DESKTOP_E2E_MCP_STDIO_COMMAND,
      );
      await setValueWhenReady(
        '[data-testid="mcp-create-stdio-args"]',
        process.env.HACHIMI_DESKTOP_E2E_MCP_STDIO_ARGS,
      );
      await setValueWhenReady(
        '[data-testid="mcp-create-stdio-cwd"]',
        process.env.HACHIMI_DESKTOP_E2E_MCP_STDIO_CWD,
      );
    } else {
      await setValueWhenReady(
        '[placeholder="https://example.com/mcp"]',
        process.env.HACHIMI_DESKTOP_E2E_MCP_URL,
      );
    }
    await clickWhenReady('[data-testid="mcp-test-new-connection"]');
    await $('[data-testid="mcp-tool-create_document"]').waitForDisplayed({ timeout: 20_000 });
    await clickWhenReady('[data-testid="mcp-save-new-server"]');
  }

  const serverToggle = await $('.mcp-detail-header [data-component="switch-root"]');
  await serverToggle.waitForDisplayed({ timeout: 20_000 });
  const serverInput = await serverToggle.$('input[type="checkbox"]');
  if (!(await serverInput.isSelected())) await serverToggle.click();
  await $('[data-testid="mcp-tool-create_document"]').waitForDisplayed({ timeout: 20_000 });
  await clickWhenReady(".back-home");
  await $('[data-testid="workbench-task-tab"]').waitForDisplayed({ timeout: 20_000 });
}

async function createGeneralTask(name, prompt, { systemNotification = false } = {}) {
  await clickWhenReady('[data-testid="task-create-toggle"]');
  await $('[data-testid="task-name"]').setValue(name);
  await $('[data-testid="task-prompt"]').setValue(prompt);
  if (systemNotification) {
    await $('[data-testid="task-delivery-policy"]').selectByAttribute(
      "value",
      "task_tab_and_system_notification",
    );
  }
  await clickWhenReady('[data-testid="task-save"]');
  await browser.waitUntil(
    async () => {
      for (const row of await $$('[data-testid="task-schedule-row"]')) {
        if ((await row.getText()).includes(name)) return true;
      }
      return false;
    },
    { timeout: 20_000, timeoutMsg: `Schedule ${name} was not created` },
  );
}

async function selectSchedule(name) {
  await browser.waitUntil(
    async () => {
      for (const row of await $$('[data-testid="task-schedule-row"]')) {
        try {
          if (
            (await row.getAttribute("aria-label")) === name ||
            (await row.getAttribute("title")) === name
          ) {
            await row.click();
            return true;
          }
        } catch {
          // The schedule projection can replace the row while a reload is
          // catching up. Retry with fresh element handles instead of
          // treating a stale WebDriver reference as a missing schedule.
        }
      }
      return false;
    },
    {
      timeout: 20_000,
      interval: 200,
      timeoutMsg: `Schedule ${name} was not found`,
    },
  );
}

async function checkOptionContaining(labelText) {
  await browser.waitUntil(
    async () => {
      for (const label of await $$("label")) {
        if (!(await label.getText()).includes(labelText)) continue;
        const input = await label.$('input[type="checkbox"]');
        if (!(await input.isExisting())) continue;
        await browser.execute((element) => {
          element.scrollIntoView({ block: "center", inline: "nearest" });
          if (!element.checked) element.click();
        }, input);
        return true;
      }
      return false;
    },
    { timeout: 20_000, timeoutMsg: `Task option ${labelText} was not available` },
  );
}

describe("Hachimi scheduled Agent tasks", () => {
  before(async () => {
    await ensureOfficeMcp();
  });

  it("creates, runs, edits and restores a General task", async () => {
    await switchToWorkbench();
    await openTaskCenter();

    const name = "Desktop E2E general schedule";
    const assertToast = process.env.HACHIMI_DESKTOP_E2E_ASSERT_TOAST === "1";
    await createGeneralTask(name, "[desktop-e2e:schedule-success] complete without side effects", {
      systemNotification: assertToast,
    });
    const revisionBefore = await $(".task-detail-meta").getText();
    await clickWhenReady('[data-testid="task-run-now"]');
    await browser.waitUntil(
      async () => (await $('[data-testid="task-run-status"]').getText()) === "succeeded",
      { timeout: 45_000, timeoutMsg: "Scheduled General Agent Run did not succeed" },
    );
    if (assertToast) assertWindowsToast(name, "已完成");

    await clickWhenReady('[data-testid="task-edit"]');
    await $('[data-testid="task-prompt"]').setValue(
      "[desktop-e2e:schedule-success] edited prompt without new authority",
    );
    await clickWhenReady('[data-testid="task-save"]');
    await expect($(".task-detail-card")).toHaveText(
      expect.stringContaining("edited prompt without new authority"),
    );
    await expect($(".task-detail-meta")).toHaveText(revisionBefore);

    await restartApplication();
    await switchToWorkbench();
    await openTaskCenter();
    await selectSchedule(name);
    await expect($('[data-testid="task-run-status"]')).toHaveText("succeeded");
  });

  it("cancels a running background Agent Run without accepting a late result", async () => {
    await openTaskCenter();
    const name = "Desktop E2E cancellation schedule";
    await createGeneralTask(name, "[desktop-e2e:schedule-wait] wait until cancellation");
    await clickWhenReady('[data-testid="task-run-now"]');
    await browser.waitUntil(
      async () => {
        const status = await $('[data-testid="task-run-status"]').getText();
        return status === "preparing" || status === "running";
      },
      { timeout: 30_000, timeoutMsg: "Scheduled Agent Run never started" },
    );
    await clickWhenReady('[data-testid="task-cancel"]');
    await browser.waitUntil(
      async () => (await $('[data-testid="task-run-status"]').getText()) === "cancelled",
      { timeout: 30_000, timeoutMsg: "Cancelled TaskRun did not reach its terminal state" },
    );
  });

  it("runs the bundled Office Skill through ordinary version-pinned MCP tools", async () => {
    await openTaskCenter();
    const name = "Desktop E2E Office Skills";
    await clickWhenReady('[data-testid="task-create-toggle"]');
    await $('[data-testid="task-name"]').setValue(name);
    await $('[data-testid="task-prompt"]').setValue(
      "[desktop-e2e:office-skills] use the document, spreadsheet, presentation, PDF and file organizer Skills to create and validate artifacts, then deliver the PDF",
    );
    for (const skill of [
      "office-documents",
      "office-spreadsheets",
      "office-presentations",
      "office-pdf",
      "office-file-organizer",
    ]) {
      await checkOptionContaining(skill);
    }
    for (const tool of [
      "create_document",
      "create_spreadsheet",
      "create_presentation",
      "create_pdf",
      "inspect_artifact",
      "modify_artifact",
      "diff_artifact",
      "export_artifact",
      "preview_file_plan",
      "send_artifact",
    ]) {
      await checkOptionContaining(tool);
    }
    await clickWhenReady('[data-testid="task-save"]');
    await browser.waitUntil(
      async () => {
        for (const row of await $$('[data-testid="task-schedule-row"]')) {
          if ((await row.getText()).includes(name)) return true;
        }
        return false;
      },
      { timeout: 20_000, timeoutMsg: "Office Schedule was not created" },
    );
    await clickWhenReady('[data-testid="task-run-now"]');
    await browser.waitUntil(
      async () => (await $('[data-testid="task-run-status"]').getText()) === "succeeded",
      { timeout: 60_000, timeoutMsg: "Office extension Agent Run did not succeed" },
    );

    for (const file of [
      "desktop-e2e-create_document.docx",
      "desktop-e2e-create_spreadsheet.xlsx",
      "desktop-e2e-create_presentation.pptx",
      "desktop-e2e-create_pdf.pdf",
      "desktop-e2e-exported.pdf",
      "desktop-e2e-artifact-diff.json",
      "desktop-e2e-file-plan.json",
      "desktop-e2e-file-rollback.json",
      "desktop-e2e-office-delivery.json",
    ]) {
      if (!existsSync(join(process.env.HACHIMI_DESKTOP_E2E_ARTIFACTS, file))) {
        throw new Error(`Expected Office E2E artifact ${file} was not created`);
      }
    }
    for (const file of [
      "desktop-e2e-create_document.docx",
      "desktop-e2e-create_spreadsheet.xlsx",
      "desktop-e2e-create_presentation.pptx",
      "desktop-e2e-create_pdf.pdf",
      "desktop-e2e-exported.pdf",
    ]) {
      validateOfficeArtifact(join(process.env.HACHIMI_DESKTOP_E2E_ARTIFACTS, file));
    }
    const filePlan = JSON.parse(
      readFileSync(
        join(process.env.HACHIMI_DESKTOP_E2E_ARTIFACTS, "desktop-e2e-file-plan.json"),
        "utf8",
      ),
    );
    if (
      filePlan.previewOnly !== true ||
      filePlan.conflictPolicy !== "suffix" ||
      filePlan.authorizedRootBoundary?.outsideRootRejected !== true
    ) {
      throw new Error("Office file organizer did not retain preview and boundary guarantees");
    }
    const delivery = JSON.parse(
      readFileSync(
        join(process.env.HACHIMI_DESKTOP_E2E_ARTIFACTS, "desktop-e2e-office-delivery.json"),
        "utf8",
      ),
    );
    if (delivery.target !== "team@example.invalid" || delivery.delivered !== true) {
      throw new Error("Office delivery did not retain the exact authorized fixture target");
    }

    await clickWhenReady('[data-testid="workbench-open-settings"]');
    await clickWhenReady('[data-testid="settings-nav-mcp"]');
    const serverName = restrictedStdioOffice
      ? "Desktop E2E Restricted Office MCP"
      : "Desktop E2E MCP";
    for (const row of await $$(".mcp-server-row")) {
      if ((await row.getText()).includes(serverName)) {
        await row.$(".mcp-server-select").click();
        break;
      }
    }
    if (!restrictedStdioOffice) {
      const schemaEndpoint = new URL("/e2e/schema-v2", process.env.HACHIMI_DESKTOP_E2E_MCP_URL);
      const schemaResponse = await fetch(schemaEndpoint, { method: "POST" });
      if (!schemaResponse.ok) throw new Error("Failed to advance the MCP schema fixture");
    }
    let serverToggle = await $('.mcp-detail-header [data-component="switch-root"]');
    if (await serverToggle.$('input[type="checkbox"]').isSelected()) await serverToggle.click();
    await browser.waitUntil(
      async () => !(await $('.mcp-detail-header input[type="checkbox"]').isSelected()),
      {
        timeout: 20_000,
        timeoutMsg: "MCP server did not stop before failure validation",
      },
    );
    if (!restrictedStdioOffice) {
      serverToggle = await $('.mcp-detail-header [data-component="switch-root"]');
      await serverToggle.click();
      await browser.waitUntil(
        async () => await $('.mcp-detail-header input[type="checkbox"]').isSelected(),
        {
          timeout: 20_000,
          timeoutMsg: "MCP server did not restart with the changed schema",
        },
      );
    }
    await clickWhenReady(".back-home");
    await openTaskCenter();
    await selectSchedule(name);
    await clickWhenReady('[data-testid="task-run-now"]');
    await browser.waitUntil(
      async () => (await $('[data-testid="task-run-status"]').getText()) === "needs_attention",
      {
        timeout: 45_000,
        timeoutMsg: restrictedStdioOffice
          ? "Interrupted stdio MCP did not enter NeedsAttention"
          : "MCP schema drift did not enter NeedsAttention",
      },
    );
    if (restrictedStdioOffice) {
      await clickWhenReady('[data-testid="workbench-open-settings"]');
      await clickWhenReady('[data-testid="settings-nav-mcp"]');
      for (const row of await $$(".mcp-server-row")) {
        if ((await row.getText()).includes(serverName)) {
          await row.$(".mcp-server-select").click();
          break;
        }
      }
      await clickWhenReady('.mcp-detail-header [data-component="switch-root"]');
      await $('[data-testid="mcp-tool-create_document"]').waitForDisplayed({ timeout: 20_000 });
      await clickWhenReady(".back-home");
      await openTaskCenter();
      await selectSchedule(name);
      await clickWhenReady('[data-testid="task-run-now"]');
      await browser.waitUntil(
        async () => (await $('[data-testid="task-run-status"]').getText()) === "succeeded",
        { timeout: 60_000, timeoutMsg: "Restarted restricted stdio MCP did not recover" },
      );
    }
    const continuation = await $(
      '//*[contains(@class, "task-run-actions")]//button[contains(., "转为交互")]',
    );
    await continuation.waitForDisplayed({ timeout: 20_000 });
    await continuation.click();
    await $(".session-timeline").waitForDisplayed({ timeout: 20_000 });
    await expect($(".session-timeline-header h1")).toHaveText(expect.stringContaining(name));
  });

  it("implicitly activates an Office Skill and recovers from a bounded resource failure", async () => {
    await switchToWorkbench();
    if (!(await $(".project-row").isExisting())) {
      await clickWhenReady('[data-testid="workbench-add-project"]');
    }
    await $(".project-row").waitForDisplayed({ timeout: 20_000 });
    await browser.execute(() => globalThis.document.querySelector(".project-row")?.focus());
    await clickWhenReady(".project-new-task");
    const prompt = await $('[data-testid="workbench-composer-input"]');
    await prompt.setValue(
      "[desktop-e2e:office-implicit-recovery] discover the document Skill, create a validated document, and recover safely if its MCP dependency fails",
    );
    await clickWhenReady('[data-testid="workbench-start-task"]');

    for (let approvals = 0; approvals < 2; approvals += 1) {
      const decision = await browser.waitUntil(
        async () => {
          try {
            if ((await $(".run-status-actions").getText()).includes("succeeded")) return "done";
            if (await $('[data-testid="workbench-approve-once"]').isDisplayed()) return "approve";
          } catch {
            // The timeline is replaced transactionally between tool steps.
          }
          return false;
        },
        {
          timeout: 150_000,
          timeoutMsg: "Implicit Office workflow stopped making progress",
        },
      );
      if (decision === "done") break;
      await clickWhenReady('[data-testid="workbench-approve-once"]');
    }
    await browser.waitUntil(
      async () => (await $(".run-status-actions").getText()).includes("succeeded"),
      { timeout: 180_000, timeoutMsg: "Implicit Office recovery Run did not succeed" },
    );
    await browser.refresh();
    await $(".session-timeline").waitForDisplayed({ timeout: 20_000 });
    await expect($(".run-status-actions")).toHaveText(expect.stringContaining("succeeded"));
    const timeline = await browser.execute(() =>
      [...globalThis.document.querySelectorAll(".session-timeline .timeline-item pre")]
        .map((item) => item.textContent ?? "")
        .join("\n"),
    );
    for (const evidence of [
      "skills.list",
      "skills.read",
      "missing-validation.md",
      "SkillHost rejected the resource read",
      "# Document validation",
      "Implicit Office Skill activated the Office overlay",
    ]) {
      if (!timeline.includes(evidence)) {
        throw new Error(`Implicit Office workflow did not project ${evidence}`);
      }
    }
  });
});
