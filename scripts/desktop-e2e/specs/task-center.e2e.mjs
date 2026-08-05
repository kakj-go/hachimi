import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { clickWhenReady, waitForDisplayed } from "../support/interactions.mjs";
import { validateOfficeArtifact } from "../support/office-artifacts.mjs";
import { restartApplication, switchToWorkbench } from "../support/windows.mjs";
import { assertWindowsToast } from "../support/windows-toast.mjs";

/* global HTMLButtonElement, HTMLElement, HTMLInputElement, HTMLTextAreaElement, XPathResult, document */

const restrictedStdioOffice = process.env.HACHIMI_DESKTOP_E2E_REAL_SANDBOX === "1";

async function readText(selector) {
  return browser.execute(
    (targetSelector) => document.querySelector(targetSelector)?.textContent ?? "",
    selector,
  );
}

async function sessionRunStatus() {
  return $('[data-testid="workbench-session-timeline"]').getAttribute("data-run-status");
}

async function waitForSessionRun(status, timeout = 60_000) {
  await browser.waitUntil(async () => (await sessionRunStatus()) === status, {
    timeout,
    timeoutMsg: `Agent Run did not reach ${status}`,
  });
}

async function readTaskRunStateFromSource() {
  const source = await browser.getPageSource();
  const row = source.match(/<div[^>]*data-testid="task-run-row"[^>]*>/)?.[0] ?? "";
  return {
    status: row.match(/data-run-status="([^"]*)"/)?.[1] ?? "",
    error: source.match(/<p[^>]*class="task-history-error"[^>]*>([^<]*)<\/p>/)?.[1] ?? "",
  };
}

function scheduleCardSelector(name) {
  return `//*[@data-testid="task-schedule-card" and @data-schedule-name="${name}"]`;
}

async function waitForSchedule(name) {
  await waitForDisplayed(scheduleCardSelector(name));
}

async function clickScheduleAction(name, action) {
  await clickWhenReady(`${scheduleCardSelector(name)}//*[@data-testid="${action}"]`);
}

async function closeTaskHistory() {
  await clickWhenReady(
    '//*[contains(@class, "task-history-dialog")]/ancestor::*[@data-component="dialog-content"]//*[@data-component="dialog-close"]',
  );
  await $(".task-history-dialog").waitForExist({ reverse: true, timeout: 10_000 });
}

async function setValueWhenReady(selector, value) {
  const expected = String(value ?? "");
  await browser.waitUntil(
    async () =>
      browser.execute(
        (targetSelector, targetValue) => {
          const element = document.querySelector(targetSelector);
          if (
            !(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) ||
            element.disabled ||
            element.readOnly ||
            element.offsetParent === null
          ) {
            return false;
          }
          element.scrollIntoView({ block: "center", inline: "nearest" });
          const prototype =
            element instanceof HTMLTextAreaElement
              ? HTMLTextAreaElement.prototype
              : HTMLInputElement.prototype;
          const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
          if (!setter) return false;
          // WebView2 intermittently rejects WebDriver's clear endpoint for a
          // visible controlled input. The native setter and bubbling events
          // follow the same state-update path as a user edit without that API.
          setter.call(element, targetValue);
          const view = element.ownerDocument.defaultView;
          if (!view) return false;
          element.dispatchEvent(new view.Event("input", { bubbles: true }));
          element.dispatchEvent(new view.Event("change", { bubbles: true }));
          const current = document.querySelector(targetSelector);
          return (
            (current instanceof HTMLInputElement || current instanceof HTMLTextAreaElement) &&
            current.value === targetValue
          );
        },
        selector,
        expected,
      ),
    { timeout: 20_000, timeoutMsg: `Input did not become editable: ${selector}` },
  );
}

async function openTaskCenter() {
  await clickWhenReady('[data-testid="workbench-task-tab"]');
  await waitForDisplayed('[data-testid="workbench-task-center"]');
}

async function ensureOfficeMcp() {
  const serverName = restrictedStdioOffice
    ? "Desktop E2E Restricted Office MCP"
    : "Desktop E2E MCP";
  await switchToWorkbench();
  const settingsVisible = await browser.execute(() => {
    const settings = document.querySelector(".settings-nav");
    return settings instanceof HTMLElement && settings.offsetParent !== null;
  });
  if (!settingsVisible) {
    await clickWhenReady('[data-testid="workbench-open-settings"]');
    await waitForDisplayed(".settings-nav");
  }
  await clickWhenReady('[data-testid="settings-nav-mcp"]');
  await waitForDisplayed('[data-testid="mcp-settings-page"]');
  await browser.waitUntil(
    () =>
      browser.execute(() => document.querySelector(".extension-list > .extension-empty") === null),
    { timeout: 20_000, timeoutMsg: "Persisted MCP server catalog did not finish loading" },
  );

  const existing = await browser.execute(
    (name) =>
      [...document.querySelectorAll(".mcp-server-row")].some((row) =>
        row.textContent?.includes(name),
      ),
    serverName,
  );
  if (existing) {
    await clickWhenReady(
      `//*[contains(@class, "mcp-server-row") and contains(., "${serverName}")]//*[contains(@class, "mcp-server-select")]`,
    );
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
    await waitForDisplayed('[data-testid="mcp-tool-create_document"]');
    await clickWhenReady('[data-testid="mcp-save-new-server"]');
  }

  const serverEnabled = await browser.execute(
    () =>
      document.querySelector(
        '.mcp-detail-header [data-component="switch-root"] input[type="checkbox"]',
      )?.checked ?? false,
  );
  if (!serverEnabled) {
    await clickWhenReady('.mcp-detail-header [data-component="switch-root"]');
  }
  await waitForDisplayed('[data-testid="mcp-tool-create_document"]');
  await clickWhenReady(".back-home");
  await waitForDisplayed('[data-testid="workbench-task-tab"]');
}

async function createGeneralTask(name, prompt, { systemNotification = false } = {}) {
  await clickWhenReady('[data-testid="task-create-toggle"]');
  await setValueWhenReady('[data-testid="task-name"]', name);
  await setValueWhenReady('[data-testid="task-prompt"]', prompt);
  if (systemNotification) {
    await $('[data-testid="task-delivery-policy"]').selectByAttribute(
      "value",
      "task_tab_and_system_notification",
    );
  }
  await clickWhenReady('[data-testid="task-save"]');
  await waitForSchedule(name);
}

async function selectSchedule(name) {
  await clickScheduleAction(name, "task-history");
  await waitForDisplayed('[data-testid="task-run-history"]');
}

async function checkOptionContaining(labelText) {
  await browser.waitUntil(
    async () =>
      browser.execute((text) => {
        const label = [...document.querySelectorAll("label")].find((candidate) =>
          candidate.textContent?.includes(text),
        );
        const input = label?.querySelector('input[type="checkbox"]');
        if (!(input instanceof HTMLInputElement)) return false;
        input.scrollIntoView({ block: "center", inline: "nearest" });
        if (!input.checked) input.click();
        return true;
      }, labelText),
    { timeout: 20_000, timeoutMsg: `Task option ${labelText} was not available` },
  );
}

async function selectFieldOption(triggerSelector, labelText) {
  await clickWhenReady(triggerSelector);
  await clickWhenReady(
    `//*[contains(@data-component, "select-item") and contains(., "${labelText}")]`,
  );
}

async function createContinuationSession(title) {
  await switchToWorkbench();
  const backVisible = await browser.execute(() => {
    const back = document.querySelector(".back-home");
    return back instanceof HTMLElement && back.offsetParent !== null;
  });
  if (backVisible) await clickWhenReady(".back-home");
  const projectExists = await browser.execute(
    () => document.querySelector(".project-row") !== null,
  );
  if (!projectExists) {
    await clickWhenReady('[data-testid="workbench-add-project"]');
  }
  await waitForDisplayed(".project-row");
  await browser.execute(() => globalThis.document.querySelector(".project-row")?.focus());
  await clickWhenReady(".project-new-task");
  await setValueWhenReady('[data-testid="workbench-composer-input"]', title);
  await clickWhenReady('[data-testid="workbench-start-task"]');
  await waitForSessionRun("succeeded", 45_000);
}

async function ensureScheduleConnector() {
  await clickWhenReady('[data-testid="workbench-open-settings"]');
  await clickWhenReady('[data-testid="settings-nav-plugins"]');
  await waitForDisplayed('[data-testid="settings-plugins-page"]');
  await browser.waitUntil(
    async () =>
      browser.execute(() => {
        const install = document.querySelector('[data-testid="host-domain-install-sample-crm"]');
        return install instanceof HTMLButtonElement && !install.disabled;
      }),
    { timeout: 30_000, timeoutMsg: "Plugin state did not finish loading" },
  );
  let pluginExists = await browser.execute(
    () => document.querySelector('[data-testid="host-domain-plugin-sample-crm"]') !== null,
  );
  if (!pluginExists) {
    await clickWhenReady('[data-testid="host-domain-install-sample-crm"]');
    await waitForDisplayed('[data-testid="host-domain-plugin-sample-crm"]');
    pluginExists = true;
  }
  const pluginEnabled =
    pluginExists &&
    (await browser.execute(
      () =>
        document
          .querySelector('[data-testid="host-domain-plugin-sample-crm"]')
          ?.textContent?.includes("enabled") ?? false,
    ));
  if (!pluginEnabled) {
    await clickWhenReady(
      '[data-testid="host-domain-plugin-sample-crm"] [data-component="switch-root"]',
    );
    await browser.waitUntil(
      async () =>
        browser.execute(
          () =>
            document
              .querySelector('[data-testid="host-domain-plugin-sample-crm"]')
              ?.textContent?.includes("enabled") ?? false,
        ),
      { timeout: 20_000, timeoutMsg: "sample-crm did not enable for Schedule E2E" },
    );
  }
  await clickWhenReady('[data-testid="settings-nav-connected-apps"]');
  await waitForDisplayed('[data-testid="settings-connected-apps-page"]');
  const accountXPath =
    '//*[@data-component="settings-row" and contains(., "Task Center E2E CRM")]//*[@data-testid and starts-with(@data-testid, "host-domain-connector-")]';
  const existingAccountId = await browser.execute((xpath) => {
    const account = document.evaluate(
      xpath,
      document,
      null,
      XPathResult.FIRST_ORDERED_NODE_TYPE,
      null,
    ).singleNodeValue;
    return account instanceof HTMLElement ? (account.dataset.testid ?? false) : false;
  }, accountXPath);
  if (existingAccountId) return existingAccountId;
  await setValueWhenReady('[data-testid="host-domain-connector-name"]', "Task Center E2E CRM");
  await setValueWhenReady(
    '[data-testid="host-domain-connector-secret"]',
    "task-center-ephemeral-credential",
  );
  await clickWhenReady('[data-testid="host-domain-create-sample-account"]');
  return browser.waitUntil(
    async () =>
      browser.execute((xpath) => {
        const account = document.evaluate(
          xpath,
          document,
          null,
          XPathResult.FIRST_ORDERED_NODE_TYPE,
          null,
        ).singleNodeValue;
        return account instanceof HTMLElement ? (account.dataset.testid ?? false) : false;
      }, accountXPath),
    { timeout: 20_000, timeoutMsg: "Schedule Connector account did not become available" },
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
    await clickScheduleAction(name, "task-run-now");
    await selectSchedule(name);
    await browser.waitUntil(
      async () => (await readTaskRunStateFromSource()).status === "succeeded",
      { timeout: 45_000, timeoutMsg: "Scheduled General Agent Run did not succeed" },
    );
    if (assertToast) assertWindowsToast(name, "已完成");

    await closeTaskHistory();
    await clickScheduleAction(name, "task-edit");
    await setValueWhenReady(
      '[data-testid="task-prompt"]',
      "[desktop-e2e:schedule-success] edited prompt without new authority",
    );
    await clickWhenReady('[data-testid="task-save"]');
    await browser.waitUntil(
      async () =>
        (await $(scheduleCardSelector(name)).getText()).includes(
          "edited prompt without new authority",
        ),
      { timeout: 20_000, timeoutMsg: "Edited Schedule prompt was not projected" },
    );

    await restartApplication();
    await switchToWorkbench();
    await openTaskCenter();
    await selectSchedule(name);
    await browser.waitUntil(
      async () => (await readTaskRunStateFromSource()).status === "succeeded",
      { timeout: 20_000, timeoutMsg: "Restored Schedule lost its succeeded status" },
    );
    await closeTaskHistory();
  });

  it("cancels a running background Agent Run without accepting a late result", async () => {
    await openTaskCenter();
    const name = "Desktop E2E cancellation schedule";
    await createGeneralTask(name, "[desktop-e2e:schedule-wait] wait until cancellation");
    await clickScheduleAction(name, "task-run-now");
    await selectSchedule(name);
    await browser.waitUntil(
      async () => {
        const { status } = await readTaskRunStateFromSource();
        return status === "preparing" || status === "running";
      },
      { timeout: 30_000, timeoutMsg: "Scheduled Agent Run never started" },
    );
    await clickWhenReady('[data-testid="task-cancel"]');
    await browser.waitUntil(
      async () => (await readTaskRunStateFromSource()).status === "cancelled",
      { timeout: 30_000, timeoutMsg: "Cancelled TaskRun did not reach its terminal state" },
    );
    await closeTaskHistory();
  });

  it("creates an Event task and projects accepted, replayed, and conflict receipts", async () => {
    await openTaskCenter();
    const name = "Desktop E2E event schedule";
    await clickWhenReady('[data-testid="task-create-toggle"]');
    await setValueWhenReady('[data-testid="task-name"]', name);
    await selectFieldOption('[data-testid="task-frequency"]', "Event");
    await setValueWhenReady('[data-testid="task-event-source-principal"]', "window:workbench");
    await setValueWhenReady('[data-testid="task-event-source-id"]', "desktop-e2e-workspace");
    await setValueWhenReady('[data-testid="task-event-type"]', "workspace.changed");
    await setValueWhenReady(
      '[data-testid="task-prompt"]',
      "[desktop-e2e:schedule-success] inspect the typed resource reference",
    );
    await clickWhenReady('[data-testid="task-save"]');
    await waitForSchedule(name);
    await expect($(scheduleCardSelector(name))).toHaveText(
      expect.stringMatching(/Waiting for event|等待匹配事件/),
    );
    await selectSchedule(name);
    await clickWhenReady('//button[contains(., "触发事件") or contains(., "Events")]');
    await expect($('[data-testid="task-event-history"]')).toBeDisplayed();

    const request = {
      context: {
        requestId: crypto.randomUUID(),
        clientId: "window:workbench",
        protocolVersion: 31,
        idempotencyKey: crypto.randomUUID(),
        expectedRunId: null,
        expectedGeneration: null,
      },
      sourceKind: "workspace",
      sourceId: "desktop-e2e-workspace",
      eventId: `desktop-e2e-event-${Date.now()}`,
      eventType: "workspace.changed",
      subject: "resource://workspace/readme",
      labels: {},
      resource: { kind: "workspace_file", id: "README.md", revision: null },
      occurredAtMs: Date.now(),
    };
    const invokeEvent = (value) =>
      browser.executeAsync((payload, done) => {
        window.__TAURI_INTERNALS__.invoke("ingest_schedule_event", { request: payload }).then(
          (receipt) => done({ receipt }),
          (error) => done({ error: String(error) }),
        );
      }, value);
    const accepted = await invokeEvent(request);
    if (accepted.error) throw new Error(`Event ingress failed: ${accepted.error}`);
    expect(accepted.receipt.status).toBe("accepted");
    expect(accepted.receipt.taskRuns).toHaveLength(1);
    const replayed = await invokeEvent({
      ...request,
      context: {
        ...request.context,
        requestId: crypto.randomUUID(),
        idempotencyKey: crypto.randomUUID(),
      },
    });
    expect(replayed.receipt.status).toBe("replayed");
    const conflictRequest = {
      ...request,
      context: {
        ...request.context,
        requestId: crypto.randomUUID(),
        idempotencyKey: crypto.randomUUID(),
      },
      subject: "resource://workspace/conflict",
    };
    const conflict = await invokeEvent(conflictRequest);
    if (conflict.error) throw new Error(`Event conflict projection failed: ${conflict.error}`);
    expect(conflict.receipt.status).toBe("conflict");
    await browser.waitUntil(
      () =>
        browser.execute(() => document.querySelector('[data-receipt-status="conflict"]') !== null),
      { timeout: 20_000, timeoutMsg: "Task Center did not project Event conflict receipt" },
    );
    await closeTaskHistory();
  });

  it("runs the bundled Office Skill through ordinary version-pinned MCP tools", async () => {
    await openTaskCenter();
    const name = "Desktop E2E Office Skills";
    await clickWhenReady('[data-testid="task-create-toggle"]');
    await setValueWhenReady('[data-testid="task-name"]', name);
    await setValueWhenReady(
      '[data-testid="task-prompt"]',
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
    await waitForSchedule(name);
    await clickScheduleAction(name, "task-run-now");
    await selectSchedule(name);
    await browser.waitUntil(
      async () => (await readTaskRunStateFromSource()).status === "succeeded",
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

    await closeTaskHistory();
    await clickWhenReady('[data-testid="workbench-open-settings"]');
    await clickWhenReady('[data-testid="settings-nav-mcp"]');
    const serverName = restrictedStdioOffice
      ? "Desktop E2E Restricted Office MCP"
      : "Desktop E2E MCP";
    await clickWhenReady(
      `//*[contains(@class, "mcp-server-row") and contains(., "${serverName}")]//*[contains(@class, "mcp-server-select")]`,
    );
    if (!restrictedStdioOffice) {
      const schemaEndpoint = new URL("/e2e/schema-v2", process.env.HACHIMI_DESKTOP_E2E_MCP_URL);
      const schemaResponse = await fetch(schemaEndpoint, { method: "POST" });
      if (!schemaResponse.ok) throw new Error("Failed to advance the MCP schema fixture");
    }
    const serverEnabled = await browser.execute(
      () => document.querySelector('.mcp-detail-header input[type="checkbox"]')?.checked ?? false,
    );
    if (serverEnabled) {
      await clickWhenReady('.mcp-detail-header [data-component="switch-root"]');
    }
    await browser.waitUntil(
      async () =>
        browser.execute(
          () =>
            document.querySelector('.mcp-detail-header input[type="checkbox"]')?.checked === false,
        ),
      {
        timeout: 20_000,
        timeoutMsg: "MCP server did not stop before failure validation",
      },
    );
    if (!restrictedStdioOffice) {
      await clickWhenReady('.mcp-detail-header [data-component="switch-root"]');
      await browser.waitUntil(
        async () =>
          browser.execute(
            () =>
              document.querySelector('.mcp-detail-header input[type="checkbox"]')?.checked === true,
          ),
        {
          timeout: 20_000,
          timeoutMsg: "MCP server did not restart with the changed schema",
        },
      );
    }
    await clickWhenReady(".back-home");
    await openTaskCenter();
    await clickScheduleAction(name, "task-run-now");
    await selectSchedule(name);
    await browser.waitUntil(
      async () => (await readTaskRunStateFromSource()).status === "needs_attention",
      {
        timeout: 45_000,
        timeoutMsg: restrictedStdioOffice
          ? "Interrupted stdio MCP did not enter NeedsAttention"
          : "MCP schema drift did not enter NeedsAttention",
      },
    );
    if (restrictedStdioOffice) {
      await closeTaskHistory();
      await clickWhenReady('[data-testid="workbench-open-settings"]');
      await clickWhenReady('[data-testid="settings-nav-mcp"]');
      await clickWhenReady(
        `//*[contains(@class, "mcp-server-row") and contains(., "${serverName}")]//*[contains(@class, "mcp-server-select")]`,
      );
      await clickWhenReady('.mcp-detail-header [data-component="switch-root"]');
      await waitForDisplayed('[data-testid="mcp-tool-create_document"]');
      await clickWhenReady(".back-home");
      await openTaskCenter();
      await clickScheduleAction(name, "task-run-now");
      await selectSchedule(name);
      await browser.waitUntil(
        async () => (await readTaskRunStateFromSource()).status === "succeeded",
        { timeout: 60_000, timeoutMsg: "Restarted restricted stdio MCP did not recover" },
      );
    }
    await clickWhenReady(
      restrictedStdioOffice ? '[data-testid="task-open-session"]' : '[data-testid="task-continue"]',
    );
    const continuationState = await browser.waitUntil(
      async () =>
        browser.execute(() => {
          const timeline = document.querySelector(".session-timeline");
          if (timeline instanceof HTMLElement && timeline.offsetParent !== null) return "opened";
          const error = document.querySelector(".task-center-error");
          if (error instanceof HTMLElement && error.offsetParent !== null) {
            return `error:${error.textContent ?? ""}`;
          }
          return false;
        }),
      { timeout: 20_000, timeoutMsg: "Interactive continuation did not open" },
    );
    if (continuationState !== "opened") throw new Error(continuationState);
    await browser.waitUntil(
      async () => (await readText('[data-testid="workbench-conversation-title"]')).includes(name),
      { timeout: 20_000, timeoutMsg: "Interactive Schedule Session title was not projected" },
    );
  });

  it.skip("pins third-party Connector and Browser grants after Bundle management is productized", async () => {
    const browserFixtureUrl = process.env.HACHIMI_DESKTOP_E2E_BROWSER_URL;
    const browserFixtureOrigin = process.env.HACHIMI_DESKTOP_E2E_BROWSER_ORIGIN;
    if (!browserFixtureUrl || !browserFixtureOrigin) {
      throw new Error("Desktop E2E Browser fixture environment is missing");
    }
    const sessionTitle = "[desktop-e2e:schedule-success] Connector Browser continuation seed";
    await createContinuationSession(sessionTitle);
    const connectorTestId = await ensureScheduleConnector();
    await clickWhenReady(".back-home");
    await openTaskCenter();

    const name = "Desktop E2E Connector Browser continuation";
    await clickWhenReady('[data-testid="task-create-toggle"]');
    await setValueWhenReady('[data-testid="task-name"]', name);
    await setValueWhenReady(
      '[data-testid="task-prompt"]',
      `[desktop-e2e:schedule-hosts] search sample-crm, observe ${browserFixtureUrl}, interact with its heading, and stop the Browser`,
    );
    await selectFieldOption('[data-testid="task-context"]', "现有对话续接");
    await selectFieldOption('[data-testid="task-session-continuation"]', "Connector Browser");
    await clickWhenReady(".task-advanced-section > summary");
    await waitForDisplayed('[data-testid="task-connectors"]');
    await checkOptionContaining("search");
    await checkOptionContaining("启用无人值守 Browser");
    const origins = await $$('[data-testid="task-browser-grant"] textarea');
    await origins[0].setValue(browserFixtureOrigin);
    await origins[1].setValue(browserFixtureOrigin);
    await checkOptionContaining("act");
    await checkOptionContaining("允许已列出的私网 Origin");
    await clickWhenReady('[data-testid="task-save"]');
    await waitForSchedule(name);
    await clickScheduleAction(name, "task-run-now");
    await selectSchedule(name);
    await browser.waitUntil(
      async () => {
        const state = await readTaskRunStateFromSource();
        const status = state.status;
        if (["failed", "needs_attention", "timed_out", "cancelled"].includes(status)) {
          throw new Error(
            `Real Connector/Browser continuation ended as ${status}: ${state.error || "no error summary"}`,
          );
        }
        return status === "succeeded";
      },
      { timeout: 90_000, timeoutMsg: "Real Connector/Browser continuation did not succeed" },
    );
    await clickWhenReady('[data-testid="task-open-session"]');
    await waitForDisplayed(".session-timeline");
    const expectedTranscriptEvidence = [
      "connector_list_accounts",
      "connector_invoke",
      "browser_start",
      "browser_observe",
      "browser_act",
      "browser_stop",
      "performed",
      "Scheduled Host E2E completed sample-crm search",
    ];
    let lastTranscript = "";
    try {
      await browser.waitUntil(
        async () => {
          lastTranscript = await browser.execute(
            () => document.querySelector(".session-timeline")?.textContent ?? "",
          );
          return expectedTranscriptEvidence.every((evidence) => lastTranscript.includes(evidence));
        },
        {
          timeout: 30_000,
          timeoutMsg: "Scheduled Host transcript omitted real Connector/Browser tool evidence",
        },
      );
    } catch (error) {
      const missing = expectedTranscriptEvidence.filter(
        (evidence) => !lastTranscript.includes(evidence),
      );
      throw new Error(
        `${error.message}; missing=${JSON.stringify(missing)}; transcript=${lastTranscript.slice(0, 12_000)}`,
      );
    }

    await clickWhenReady('[data-testid="workbench-open-settings"]');
    await clickWhenReady('[data-testid="settings-nav-connected-apps"]');
    await waitForDisplayed('[data-testid="settings-connected-apps-page"]');
    await waitForDisplayed(`[data-testid="${connectorTestId}"]`);
    await clickWhenReady(`[data-testid="${connectorTestId}"] button`);
    await browser.waitUntil(
      async () => (await readText(`[data-testid="${connectorTestId}"]`)).includes("revoked"),
      { timeout: 20_000, timeoutMsg: "Connector credential revocation did not persist" },
    );
    await clickWhenReady(".back-home");
    await openTaskCenter();
    await clickScheduleAction(name, "task-run-now");
    await selectSchedule(name);
    await browser.waitUntil(
      async () => (await readTaskRunStateFromSource()).status === "needs_attention",
      { timeout: 45_000, timeoutMsg: "Revoked Connector did not enter NeedsAttention" },
    );
    await clickWhenReady('[data-testid="task-continue"]');
    const continuationState = await browser.waitUntil(
      async () =>
        browser.execute(() => {
          const timeline = document.querySelector(".session-timeline");
          if (timeline instanceof HTMLElement && timeline.offsetParent !== null) return "opened";
          const error = document.querySelector(".task-center-error");
          if (error instanceof HTMLElement && error.offsetParent !== null) {
            return `error:${error.textContent ?? ""}`;
          }
          return false;
        }),
      { timeout: 20_000, timeoutMsg: "Interactive continuation did not open" },
    );
    if (continuationState !== "opened") throw new Error(continuationState);
    await browser.waitUntil(
      async () =>
        (await readText('[data-testid="workbench-conversation-title"]')).includes(sessionTitle),
      { timeout: 20_000, timeoutMsg: "Continuation Session title was not projected" },
    );
  });

  it("implicitly activates an Office Skill and recovers from a bounded resource failure", async () => {
    await switchToWorkbench();
    const backVisible = await browser.execute(() => {
      const back = document.querySelector(".back-home");
      return back instanceof HTMLElement && back.offsetParent !== null;
    });
    if (backVisible) await clickWhenReady(".back-home");
    const projectExists = await browser.execute(
      () => document.querySelector(".project-row") !== null,
    );
    if (!projectExists) {
      await clickWhenReady('[data-testid="workbench-add-project"]');
    }
    await waitForDisplayed(".project-row");
    await browser.execute(() => globalThis.document.querySelector(".project-row")?.focus());
    await clickWhenReady(".project-new-task");
    await setValueWhenReady(
      '[data-testid="workbench-composer-input"]',
      "[desktop-e2e:office-implicit-recovery] discover the document Skill, create a validated document, and recover safely if its MCP dependency fails",
    );
    await clickWhenReady('[data-testid="workbench-start-task"]');

    for (let approvals = 0; approvals < 2; approvals += 1) {
      const decision = await browser.waitUntil(
        async () => {
          const state = await browser.execute(() => ({
            status:
              document
                .querySelector('[data-testid="workbench-session-timeline"]')
                ?.getAttribute("data-run-status") ?? "",
            approvalVisible:
              document.querySelector('[data-testid="workbench-approve-once"]') instanceof
                HTMLElement &&
              document.querySelector('[data-testid="workbench-approve-once"]')?.offsetParent !==
                null,
          }));
          if (state.status.includes("succeeded")) return "done";
          if (state.approvalVisible) return "approve";
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
    await waitForSessionRun("succeeded", 180_000);
    await browser.refresh();
    await waitForDisplayed(".session-timeline");
    await waitForSessionRun("succeeded", 20_000);
    const timeline = await browser.execute(
      () => globalThis.document.querySelector(".session-timeline")?.textContent ?? "",
    );
    for (const evidence of [
      "skills.list",
      "skills.read",
      "SkillHost rejected the resource read",
      "Implicit Office Skill activated the Office overlay",
    ]) {
      if (!timeline.includes(evidence)) {
        throw new Error(`Implicit Office workflow did not project ${evidence}`);
      }
    }
  });
});
