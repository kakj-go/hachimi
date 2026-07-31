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

async function readTaskRunStateFromSource() {
  const source = await browser.getPageSource();
  return {
    status:
      source.match(/<strong[^>]*data-testid="task-run-status"[^>]*>([^<]*)<\/strong>/)?.[1] ?? "",
    error: source.match(/<small[^>]*class="task-run-error"[^>]*>([^<]*)<\/small>/)?.[1] ?? "",
  };
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
  await browser.waitUntil(
    async () =>
      browser.execute(
        (scheduleName) =>
          [...document.querySelectorAll('[data-testid="task-schedule-row"]')].some((row) =>
            row.textContent?.includes(scheduleName),
          ),
        name,
      ),
    { timeout: 20_000, timeoutMsg: `Schedule ${name} was not created` },
  );
}

async function selectSchedule(name) {
  await clickWhenReady(
    `//*[@data-testid="task-schedule-row" and (@aria-label="${name}" or @title="${name}")]`,
  );
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
  await browser.waitUntil(
    async () => (await readText(".run-status-actions")).includes("succeeded"),
    { timeout: 45_000, timeoutMsg: "Continuation seed Session did not succeed" },
  );
}

async function ensureScheduleConnector() {
  await clickWhenReady('[data-testid="workbench-open-settings"]');
  await clickWhenReady('[data-testid="settings-nav-local-hosts"]');
  await waitForDisplayed('[data-testid="local-hosts-settings-page"]');
  await browser.waitUntil(
    async () =>
      browser.execute(() => {
        const capabilities = document.querySelector(
          '[data-testid="local-hosts-sandbox-capabilities"]',
        );
        const install = document.querySelector('[data-testid="local-hosts-install-sample-crm"]');
        return (
          capabilities?.textContent === "4/4" &&
          install instanceof HTMLButtonElement &&
          !install.disabled
        );
      }),
    { timeout: 30_000, timeoutMsg: "Local Host state did not finish loading" },
  );
  let pluginExists = await browser.execute(
    () => document.querySelector('[data-testid="local-hosts-plugin-sample-crm"]') !== null,
  );
  if (!pluginExists) {
    await clickWhenReady('[data-testid="local-hosts-install-sample-crm"]');
    await waitForDisplayed('[data-testid="local-hosts-plugin-sample-crm"]');
    pluginExists = true;
  }
  const pluginEnabled =
    pluginExists &&
    (await browser.execute(
      () =>
        document
          .querySelector('[data-testid="local-hosts-plugin-sample-crm"]')
          ?.textContent?.includes("enabled") ?? false,
    ));
  if (!pluginEnabled) {
    await clickWhenReady(
      '[data-testid="local-hosts-plugin-sample-crm"] [data-component="switch-root"]',
    );
    await browser.waitUntil(
      async () =>
        browser.execute(
          () =>
            document
              .querySelector('[data-testid="local-hosts-plugin-sample-crm"]')
              ?.textContent?.includes("enabled") ?? false,
        ),
      { timeout: 20_000, timeoutMsg: "sample-crm did not enable for Schedule E2E" },
    );
  }
  const accountXPath =
    '//*[@data-component="settings-row" and contains(., "Task Center E2E CRM")]//*[@data-testid and starts-with(@data-testid, "local-hosts-connector-")]';
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
  await setValueWhenReady('[data-testid="local-hosts-connector-name"]', "Task Center E2E CRM");
  await setValueWhenReady(
    '[data-testid="local-hosts-connector-secret"]',
    "task-center-ephemeral-credential",
  );
  await clickWhenReady('[data-testid="local-hosts-create-sample-account"]');
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
    const revisionBefore = await readText(".task-detail-meta");
    await clickWhenReady('[data-testid="task-run-now"]');
    await browser.waitUntil(
      async () => (await readText('[data-testid="task-run-status"]')) === "succeeded",
      { timeout: 45_000, timeoutMsg: "Scheduled General Agent Run did not succeed" },
    );
    if (assertToast) assertWindowsToast(name, "已完成");

    await clickWhenReady('[data-testid="task-edit"]');
    await setValueWhenReady(
      '[data-testid="task-prompt"]',
      "[desktop-e2e:schedule-success] edited prompt without new authority",
    );
    await clickWhenReady('[data-testid="task-save"]');
    await browser.waitUntil(
      async () =>
        (await readText(".task-detail-card")).includes("edited prompt without new authority"),
      { timeout: 20_000, timeoutMsg: "Edited Schedule prompt was not projected" },
    );
    await browser.waitUntil(async () => (await readText(".task-detail-meta")) === revisionBefore, {
      timeout: 20_000,
      timeoutMsg: "Schedule revision metadata changed unexpectedly",
    });

    await restartApplication();
    await switchToWorkbench();
    await openTaskCenter();
    await selectSchedule(name);
    await browser.waitUntil(
      async () => (await readText('[data-testid="task-run-status"]')) === "succeeded",
      { timeout: 20_000, timeoutMsg: "Restored Schedule lost its succeeded status" },
    );
  });

  it("cancels a running background Agent Run without accepting a late result", async () => {
    await openTaskCenter();
    const name = "Desktop E2E cancellation schedule";
    await createGeneralTask(name, "[desktop-e2e:schedule-wait] wait until cancellation");
    await clickWhenReady('[data-testid="task-run-now"]');
    await browser.waitUntil(
      async () => {
        const status = await readText('[data-testid="task-run-status"]');
        return status === "preparing" || status === "running";
      },
      { timeout: 30_000, timeoutMsg: "Scheduled Agent Run never started" },
    );
    await clickWhenReady('[data-testid="task-cancel"]');
    await browser.waitUntil(
      async () => (await readText('[data-testid="task-run-status"]')) === "cancelled",
      { timeout: 30_000, timeoutMsg: "Cancelled TaskRun did not reach its terminal state" },
    );
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
    await selectSchedule(name);
    await expect($('[data-testid="task-event-history"]')).toBeDisplayed();
    expect(await readText(".task-detail-meta")).toMatch(/Waiting for matching event|等待匹配事件/);

    const request = {
      context: {
        requestId: crypto.randomUUID(),
        clientId: "window:workbench",
        protocolVersion: 29,
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
      async () => (await readText('[data-testid="task-event-history"]')).includes("conflict"),
      { timeout: 20_000, timeoutMsg: "Task Center did not project Event conflict receipt" },
    );
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
    await browser.waitUntil(
      async () =>
        browser.execute(
          (scheduleName) =>
            [...document.querySelectorAll('[data-testid="task-schedule-row"]')].some((row) =>
              row.textContent?.includes(scheduleName),
            ),
          name,
        ),
      { timeout: 20_000, timeoutMsg: "Office Schedule was not created" },
    );
    await clickWhenReady('[data-testid="task-run-now"]');
    await browser.waitUntil(
      async () => (await readText('[data-testid="task-run-status"]')) === "succeeded",
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
    await selectSchedule(name);
    await clickWhenReady('[data-testid="task-run-now"]');
    await browser.waitUntil(
      async () => (await readText('[data-testid="task-run-status"]')) === "needs_attention",
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
      await clickWhenReady(
        `//*[contains(@class, "mcp-server-row") and contains(., "${serverName}")]//*[contains(@class, "mcp-server-select")]`,
      );
      await clickWhenReady('.mcp-detail-header [data-component="switch-root"]');
      await waitForDisplayed('[data-testid="mcp-tool-create_document"]');
      await clickWhenReady(".back-home");
      await openTaskCenter();
      await selectSchedule(name);
      await clickWhenReady('[data-testid="task-run-now"]');
      await browser.waitUntil(
        async () => (await readText('[data-testid="task-run-status"]')) === "succeeded",
        { timeout: 60_000, timeoutMsg: "Restarted restricted stdio MCP did not recover" },
      );
    }
    await clickWhenReady(
      '//*[contains(@class, "task-run-actions")]//button[contains(., "转为交互")]',
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
      async () => (await readText(".session-timeline-header h1")).includes(name),
      { timeout: 20_000, timeoutMsg: "Interactive Schedule Session title was not projected" },
    );
  });

  it("pins Connector and Browser grants on a Session continuation and detects revocation", async () => {
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
      `[desktop-e2e:schedule-hosts] search sample-crm, observe ${browserFixtureUrl}, download its fixture, and stop the Browser`,
    );
    await selectFieldOption('[data-testid="task-context"]', "现有对话续接");
    await selectFieldOption('[data-testid="task-session-continuation"]', "Connector Browser");
    await waitForDisplayed('[data-testid="task-connectors"]');
    await checkOptionContaining("search");
    await checkOptionContaining("启用无人值守 Browser");
    const origins = await $$('[data-testid="task-browser-grant"] textarea');
    await origins[0].setValue(browserFixtureOrigin);
    await origins[1].setValue(browserFixtureOrigin);
    await checkOptionContaining("download");
    await checkOptionContaining("允许已列出的私网 Origin");
    await clickWhenReady('[data-testid="task-save"]');
    await selectSchedule(name);
    await clickWhenReady('[data-testid="task-run-now"]');
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
    await clickWhenReady(
      '//*[@data-testid="task-run-row"]//button[contains(., "查看") or contains(., "Open")]',
    );
    await waitForDisplayed(".session-timeline");
    const expectedTranscriptEvidence = [
      "connector_list_accounts",
      "connector_invoke",
      "browser_start",
      "browser_observe",
      "browser_act",
      "browser_stop",
      "download_quarantined",
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
    await clickWhenReady('[data-testid="settings-nav-local-hosts"]');
    await waitForDisplayed(`[data-testid="${connectorTestId}"]`);
    await clickWhenReady(`[data-testid="${connectorTestId}"] button`);
    await browser.waitUntil(
      async () => (await readText(`[data-testid="${connectorTestId}"]`)).includes("revoked"),
      { timeout: 20_000, timeoutMsg: "Connector credential revocation did not persist" },
    );
    await clickWhenReady(".back-home");
    await openTaskCenter();
    await selectSchedule(name);
    await clickWhenReady('[data-testid="task-run-now"]');
    await browser.waitUntil(
      async () => (await readText('[data-testid="task-run-status"]')) === "needs_attention",
      { timeout: 45_000, timeoutMsg: "Revoked Connector did not enter NeedsAttention" },
    );
    await clickWhenReady(
      '//*[contains(@class, "task-run-actions")]//button[contains(., "转为交互")]',
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
      async () => (await readText(".session-timeline-header h1")).includes(sessionTitle),
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
            status: document.querySelector(".run-status-actions")?.textContent ?? "",
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
    await browser.waitUntil(
      async () => (await readText(".run-status-actions")).includes("succeeded"),
      { timeout: 180_000, timeoutMsg: "Implicit Office recovery Run did not succeed" },
    );
    await browser.refresh();
    await waitForDisplayed(".session-timeline");
    await browser.waitUntil(
      async () => (await readText(".run-status-actions")).includes("succeeded"),
      { timeout: 20_000, timeoutMsg: "Recovered Office Session lost its succeeded status" },
    );
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
