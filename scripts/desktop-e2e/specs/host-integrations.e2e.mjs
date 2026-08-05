import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { join, resolve } from "node:path";

import { clickWhenReady, waitForDisplayed } from "../support/interactions.mjs";
import { cleanupGatewayProcesses, terminateProcessTree } from "../support/processes.mjs";
import { restartApplication, switchToWorkbench } from "../support/windows.mjs";

/* global HTMLElement, document, getComputedStyle */

let connectorTestId;
const gatewayBaseUrl = `http://127.0.0.1:${process.env.HACHIMI_DESKTOP_E2E_GATEWAY_PORT ?? "42371"}`;

async function openHostSettings(section) {
  await switchToWorkbench();
  const settingsVisible = await browser.execute(() => {
    const settings = document.querySelector(".settings-nav");
    if (!(settings instanceof HTMLElement)) return false;
    return getComputedStyle(settings).visibility !== "hidden" && settings.offsetParent !== null;
  });
  if (!settingsVisible) {
    await clickWhenReady('[data-testid="workbench-open-settings"]');
    await waitForDisplayed(".settings-nav");
  }
  await clickWhenReady(`[data-testid="settings-nav-${section}"]`);
  await waitForDisplayed(`[data-testid="settings-${section}-page"]`);
}

async function textFieldByLabel(label) {
  for (const field of await $$('[data-component="form-field"]')) {
    const fieldLabel = await field.$('[data-component="form-label"]');
    if ((await fieldLabel.getText()) === label) return field.$("input");
  }
  throw new Error(`Text field not found: ${label}`);
}

async function waitForIntegrationAccount(providerId, displayName, present, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const failure = await $('[data-testid="integration-failure"]');
    if ((await failure.isExisting()) && (await failure.isDisplayed())) {
      throw new Error(`Platform integration failed: ${await failure.getText()}`);
    }
    const text = await $(`[data-testid="integration-provider-${providerId}"]`).getText();
    if (text.includes(displayName) === present) return;
    await browser.pause(100);
  }
  throw new Error(
    present
      ? `Enterprise integration account was not created: ${displayName}`
      : `Enterprise integration account was not removed: ${displayName}`,
  );
}

async function samplePluginRow() {
  await waitForDisplayed('[data-testid="host-domain-plugin-sample-crm"]');
  return $('[data-testid="host-domain-plugin-sample-crm"]');
}

async function waitForJson(url, predicate, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        const value = await response.json();
        const selected = predicate(value);
        if (selected) return selected;
      }
    } catch {
      // Managed Chrome and its extension service worker are still starting.
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
  }
  throw new Error(`Timed out waiting for ${url}`);
}

async function cdpCommand(webSocketUrl, method, params = {}, timeoutMs = 10_000) {
  return new Promise((resolveResult, rejectResult) => {
    const socket = new WebSocket(webSocketUrl);
    const timer = setTimeout(() => {
      socket.close();
      rejectResult(new Error(`CDP ${method} timed out`));
    }, timeoutMs);
    socket.addEventListener("open", () => {
      socket.send(
        JSON.stringify({
          id: 1,
          method,
          params,
        }),
      );
    });
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(String(event.data));
      if (message.id !== 1) return;
      clearTimeout(timer);
      socket.close();
      if (message.error || message.result?.exceptionDetails) {
        rejectResult(
          new Error(
            message.error?.message ??
              message.result?.exceptionDetails?.exception?.description ??
              "CDP expression failed",
          ),
        );
      } else {
        resolveResult(message.result);
      }
    });
    socket.addEventListener("error", () => {
      clearTimeout(timer);
      rejectResult(new Error("CDP WebSocket failed"));
    });
  });
}

async function cdpEvaluate(webSocketUrl, expression, timeoutMs = 10_000) {
  const result = await cdpCommand(
    webSocketUrl,
    "Runtime.evaluate",
    {
      expression,
      returnByValue: true,
      awaitPromise: true,
    },
    timeoutMs,
  );
  return result?.result?.value;
}

async function reserveLoopbackPort() {
  const server = createServer();
  await new Promise((resolveListen, rejectListen) => {
    server.once("error", rejectListen);
    server.listen(0, "127.0.0.1", resolveListen);
  });
  const address = server.address();
  if (!address || typeof address === "string") {
    server.close();
    throw new Error("Chrome debugging port allocation failed");
  }
  await new Promise((resolveClose, rejectClose) =>
    server.close((error) => (error ? rejectClose(error) : resolveClose())),
  );
  return address.port;
}

async function pairRealChromeExtension(nonce) {
  const chrome = resolve("apps/desktop/src-tauri/managed-chromium/chrome.exe");
  const extension = resolve("assets/browser-extension");
  const browserFixtureUrl = process.env.HACHIMI_DESKTOP_E2E_BROWSER_URL;
  const browserFixtureOrigin = process.env.HACHIMI_DESKTOP_E2E_BROWSER_ORIGIN;
  const uploadFixture = process.env.HACHIMI_DESKTOP_E2E_ATTACHMENT_PATH;
  if (!browserFixtureUrl || !browserFixtureOrigin || !uploadFixture) {
    throw new Error("Chrome extension file-transfer fixture environment is missing");
  }
  const profile = mkdtempSync(join(process.env.HACHIMI_DATA_DIR, "chrome-extension-e2e-"));
  const debugPort = await reserveLoopbackPort();
  const child = spawn(
    chrome,
    [
      `--user-data-dir=${profile}`,
      "--enable-unsafe-extension-debugging",
      `--remote-debugging-port=${debugPort}`,
      "--remote-allow-origins=*",
      "--no-first-run",
      "--no-default-browser-check",
      "about:blank",
    ],
    { stdio: "ignore", windowsHide: true },
  );
  let extensionId;
  let cleanupError;
  try {
    const browserTarget = await waitForJson(
      `http://127.0.0.1:${debugPort}/json/version`,
      (version) => version.webSocketDebuggerUrl && version,
    );
    const loaded = await cdpCommand(browserTarget.webSocketDebuggerUrl, "Extensions.loadUnpacked", {
      path: extension,
    });
    extensionId = loaded?.id;
    if (!extensionId) {
      throw new Error("Chrome did not return an extension id after loading the bundled extension");
    }
    const serviceWorker = await waitForJson(`http://127.0.0.1:${debugPort}/json/list`, (targets) =>
      targets.find(
        (target) =>
          target.type === "service_worker" &&
          target.url === `chrome-extension://${extensionId}/service-worker.js`,
      ),
    );
    const popupUrl = `chrome-extension://${extensionId}/popup.html`;
    await cdpCommand(browserTarget.webSocketDebuggerUrl, "Target.createTarget", {
      url: popupUrl,
    });
    const popup = await waitForJson(`http://127.0.0.1:${debugPort}/json/list`, (targets) =>
      targets.find((target) => target.type === "page" && target.url === popupUrl),
    );
    const popupDeadline = Date.now() + 10_000;
    let popupLoaded = false;
    while (Date.now() < popupDeadline) {
      popupLoaded = Boolean(
        await cdpEvaluate(
          popup.webSocketDebuggerUrl,
          'document.readyState === "complete" && Boolean(document.querySelector("#nonce"))',
        ),
      );
      if (popupLoaded) break;
      await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
    }
    if (!popupLoaded) {
      const snapshot = await cdpEvaluate(
        popup.webSocketDebuggerUrl,
        "JSON.stringify({ href: location.href, title: document.title, text: document.body?.innerText ?? '' })",
      );
      throw new Error(
        `Chrome extension popup did not load: ${snapshot}; worker=${serviceWorker.url}`,
      );
    }
    const extensionDiagnostic = await cdpEvaluate(
      popup.webSocketDebuggerUrl,
      `(() => {
        const manifest = chrome.runtime.getManifest();
        const url = chrome.runtime.getURL("popup.html");
        return JSON.stringify({ name: manifest.name, popup: manifest.action?.default_popup, url });
      })()`,
    );
    if (!String(extensionDiagnostic).includes(popupUrl)) {
      throw new Error(`Chrome extension identity mismatch: ${extensionDiagnostic}`);
    }
    await cdpEvaluate(
      popup.webSocketDebuggerUrl,
      `document.querySelector("#nonce").value = ${JSON.stringify(nonce)}; document.querySelector("#pair").click();`,
    );
    const pairingDeadline = Date.now() + 20_000;
    let paired = false;
    while (Date.now() < pairingDeadline) {
      const status = String(
        await cdpEvaluate(
          popup.webSocketDebuggerUrl,
          'document.querySelector("#status").textContent',
        ),
      );
      if (status.startsWith("Paired.")) {
        paired = true;
        break;
      }
      if (status.includes("failed") || status.includes("rejected")) {
        throw new Error(`Chrome extension pairing failed: ${status}`);
      }
      await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
    }
    if (!paired) throw new Error("Chrome extension pairing did not complete");

    let lifecycleText;
    try {
      lifecycleText = await cdpEvaluate(
        popup.webSocketDebuggerUrl,
        `(async () => {
          globalThis.__hachimiE2EPhase = "runtime_import";
          const runtime = await import(chrome.runtime.getURL("service-worker.js"));
          const waitForTab = async (tabId) => {
            for (let attempt = 0; attempt < 200; attempt += 1) {
              const tab = await chrome.tabs.get(tabId);
              if (tab.status === "complete") return tab;
              await new Promise((resolve) => setTimeout(resolve, 100));
            }
            throw new Error("extension_tab_load_timeout");
          };
          const sessionId = "desktop-e2e-extension-session";
          await runtime.execute({
            commandId: "desktop-e2e-start",
            kind: "start",
            sessionId,
            taskTabGroup: "Hachimi Desktop E2E",
            initialUrl: null,
            networkPolicy: {
              revision: 7,
              rules: [
                { origin: "https://example.com", kind: "document", expiresAtMs: null },
                { origin: "https://cdn.example.com", kind: "resource", expiresAtMs: null },
              ],
            },
          });
          globalThis.__hachimiE2EPhase = "initial_started";
          const started = (await runtime.sessions())[sessionId];
          const group = await chrome.tabGroups.get(started.groupId);
          const rules = await chrome.declarativeNetRequest.getSessionRules();
          const ownedRules = rules.filter((rule) => started.networkRuleIds.includes(rule.id));
          const documentRule = ownedRules.find((rule) => rule.condition.regexFilter?.includes("example\\\\.com"));
          const resourceRule = ownedRules.find((rule) => rule.condition.regexFilter?.includes("cdn\\\\.example\\\\.com"));
          await runtime.execute({ commandId: "desktop-e2e-take-over", kind: "take_over", sessionId });
          globalThis.__hachimiE2EPhase = "initial_taken_over";
          const released = (await runtime.sessions())[sessionId];
          const remaining = await chrome.declarativeNetRequest.getSessionRules();
          const transferSessionId = "desktop-e2e-extension-file-transfer";
          await runtime.execute({
            commandId: "desktop-e2e-transfer-start",
            kind: "start",
            sessionId: transferSessionId,
            taskTabGroup: "Hachimi File Transfer E2E",
            initialUrl: ${JSON.stringify(browserFixtureUrl)},
            networkPolicy: {
              revision: 11,
              rules: [
                { origin: ${JSON.stringify(browserFixtureOrigin)}, kind: "document", expiresAtMs: null },
                { origin: ${JSON.stringify(browserFixtureOrigin)}, kind: "resource", expiresAtMs: null },
              ],
            },
          });
          globalThis.__hachimiE2EPhase = "transfer_started";
          const transfer = (await runtime.sessions())[transferSessionId];
          await waitForTab(transfer.tabId);
          const upload = await runtime.execute({
            commandId: "desktop-e2e-extension-upload",
            kind: "act",
            sessionId: transferSessionId,
            expectedOrigin: ${JSON.stringify(browserFixtureOrigin)},
            action: {
              kind: "upload",
              selector: "#upload",
              file_token: ${JSON.stringify(uploadFixture)},
            },
          });
          globalThis.__hachimiE2EPhase = "upload_completed";
          const uploadedFile = (await chrome.scripting.executeScript({
            target: { tabId: transfer.tabId },
            func: () => {
              const file = document.querySelector("#upload")?.files?.[0];
              return file ? { name: file.name, size: file.size } : null;
            },
          }))[0]?.result;
          const rogue = await chrome.tabs.create({ url: ${JSON.stringify(browserFixtureUrl)}, active: false });
          await waitForTab(rogue.id);
          const rogueBefore = new Set((await chrome.downloads.search({ limit: 100 })).map((item) => item.id));
          await chrome.scripting.executeScript({
            target: { tabId: rogue.id },
            func: () => document.querySelector("#download").click(),
          });
          globalThis.__hachimiE2EPhase = "rogue_download_started";
          let rogueDownload = null;
          for (let attempt = 0; attempt < 200; attempt += 1) {
            rogueDownload = (await chrome.downloads.search({ limit: 100 })).find(
              (item) => !rogueBefore.has(item.id) && item.state === "complete" && !item.error,
            );
            if (rogueDownload) break;
            await new Promise((resolve) => setTimeout(resolve, 100));
          }
          if (!rogueDownload) throw new Error("extension_cross_tab_fixture_missing");
          globalThis.__hachimiE2EPhase = "rogue_download_completed";
          const download = await runtime.execute({
            commandId: "desktop-e2e-extension-download",
            kind: "act",
            sessionId: transferSessionId,
            expectedOrigin: ${JSON.stringify(browserFixtureOrigin)},
            action: { kind: "download", selector: "#download", allow_unknown_type: false },
          });
          globalThis.__hachimiE2EPhase = "owned_download_completed";
          await runtime.execute({
            commandId: "desktop-e2e-transfer-stop",
            kind: "stop",
            sessionId: transferSessionId,
          });
          globalThis.__hachimiE2EPhase = "transfer_stopped";
          const transferStopped = !(transferSessionId in (await runtime.sessions()));
          const rulesAfterStop = await chrome.declarativeNetRequest.getSessionRules();
          await chrome.tabs.remove(rogue.id);
          return JSON.stringify({
            groupTitle: group.title,
            tabId: started.tabId,
            ruleCount: ownedRules.length,
            documentAllowsMainFrame: documentRule?.condition.resourceTypes?.includes("main_frame") ?? false,
            resourceAllowsMainFrame: resourceRule?.condition.resourceTypes?.includes("main_frame") ?? false,
            ownedAfterTakeOver: released.owned,
            ruleCountAfterTakeOver: remaining.filter((rule) => started.networkRuleIds.includes(rule.id)).length,
            uploadResult: upload.action?.resultCode,
            uploadedFile,
            downloadResult: download.action?.resultCode,
            downloadId: download.action?.output?.downloadId,
            downloadGuid: download.action?.output?.downloadGuid,
            ownerTabId: download.action?.output?.ownerTabId,
            rogueDownloadId: rogueDownload.id,
            transferTabId: transfer.tabId,
            transferStopped,
            transferRulesAfterStop: rulesAfterStop.filter((rule) => transfer.networkRuleIds.includes(rule.id)).length,
          });
        })()`,
        60_000,
      );
    } catch (error) {
      const phase = await cdpEvaluate(
        popup.webSocketDebuggerUrl,
        "globalThis.__hachimiE2EPhase ?? 'unknown'",
      ).catch(() => "unavailable");
      const downloads = await cdpEvaluate(
        popup.webSocketDebuggerUrl,
        "chrome.downloads.search({ limit: 10 }).then((items) => JSON.stringify(items.map(({ id, tabId, state, error, filename, url }) => ({ id, tabId, state, error, filename, url }))))",
      ).catch(() => "unavailable");
      throw new Error(`${error.message}; phase=${phase}; downloads=${downloads}`);
    }
    const lifecycle = JSON.parse(lifecycleText);
    if (
      lifecycle.groupTitle !== "Hachimi Desktop E2E" ||
      !Number.isInteger(lifecycle.tabId) ||
      lifecycle.ruleCount !== 3 ||
      !lifecycle.documentAllowsMainFrame ||
      lifecycle.resourceAllowsMainFrame ||
      lifecycle.ownedAfterTakeOver ||
      lifecycle.ruleCountAfterTakeOver !== 0 ||
      lifecycle.uploadResult !== "uploaded" ||
      lifecycle.uploadedFile?.name !== "reference.txt" ||
      lifecycle.uploadedFile?.size <= 0 ||
      lifecycle.downloadResult !== "download_quarantined" ||
      lifecycle.downloadId === lifecycle.rogueDownloadId ||
      typeof lifecycle.downloadGuid !== "string" ||
      lifecycle.ownerTabId !== lifecycle.transferTabId ||
      !lifecycle.transferStopped ||
      lifecycle.transferRulesAfterStop !== 0
    ) {
      throw new Error(`Chrome extension task fencing failed: ${JSON.stringify(lifecycle)}`);
    }
  } finally {
    terminateProcessTree(child.pid);
    try {
      rmSync(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
    } catch (error) {
      if (error?.code !== "EPERM" && error?.code !== "EBUSY") cleanupError = error;
      // Chrome can retain a short-lived Windows profile handle after taskkill.
      // The runner owns HACHIMI_DATA_DIR and removes it after the WebDriver exits.
    }
  }
  if (cleanupError) throw cleanupError;
  return extensionId;
}

async function enableGatewayStartup() {
  const toggleSelector = '[data-testid="host-domain-gateway-startup"]';
  await waitForDisplayed(toggleSelector);
  const selected = await browser.execute(
    (selector) => document.querySelector(`${selector} input[type="checkbox"]`)?.checked ?? false,
    toggleSelector,
  );
  if (!selected) {
    await clickWhenReady(toggleSelector);
  }
  await waitForGatewayEndpoint();
}

async function disableGatewayStartup() {
  const toggleSelector = '[data-testid="host-domain-gateway-startup"]';
  await waitForDisplayed(toggleSelector);
  const selected = await browser.execute(
    (selector) => document.querySelector(`${selector} input[type="checkbox"]`)?.checked ?? false,
    toggleSelector,
  );
  if (selected) {
    await clickWhenReady(toggleSelector);
  }
  await browser.waitUntil(
    async () =>
      browser.execute(
        (selector) =>
          document.querySelector(`${selector} input[type="checkbox"]`)?.checked === false,
        toggleSelector,
      ),
    {
      timeout: 20_000,
      timeoutMsg: "per-user Gateway startup registration did not turn off",
    },
  );
}

async function waitForGatewayEndpoint() {
  await browser.waitUntil(
    async () => {
      try {
        const response = await fetch(
          `${gatewayBaseUrl}/v1/channels/loopback-webhook/outbox/claim`,
          {
            method: "POST",
            headers: { Authorization: `Bearer ${process.env.HACHIMI_DESKTOP_E2E_LOOPBACK_TOKEN}` },
          },
        );
        return response.status === 204;
      } catch {
        return false;
      }
    },
    { timeout: 20_000, timeoutMsg: "per-user Gateway HTTP endpoint did not start" },
  );
}

async function waitForGatewayEndpointClosed() {
  await browser.waitUntil(
    async () => {
      try {
        await fetch(`${gatewayBaseUrl}/v1/channels/loopback-webhook/outbox/claim`, {
          method: "POST",
          headers: { Authorization: `Bearer ${process.env.HACHIMI_DESKTOP_E2E_LOOPBACK_TOKEN}` },
        });
        return false;
      } catch {
        return true;
      }
    },
    { timeout: 20_000, timeoutMsg: "per-user Gateway HTTP endpoint did not stop" },
  );
}

async function sendLoopback(messageId) {
  const token = process.env.HACHIMI_DESKTOP_E2E_LOOPBACK_TOKEN;
  const response = await fetch(`${gatewayBaseUrl}/v1/channels/loopback-webhook`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      messageId,
      route: {
        channel: "loopback-webhook",
        account: "local",
        peer: "local-user",
        thread: "main",
      },
      sender: "desktop-e2e",
      text: "Return one deterministic safe status line.",
      metadata: { fixture: true },
      authenticated: false,
      botGenerated: false,
      receivedAtMs: Date.now(),
    }),
  });
  if (response.status !== 202) throw new Error(`loopback ingress failed: ${response.status}`);
  return response.json();
}

async function claimLoopbackResult() {
  const token = process.env.HACHIMI_DESKTOP_E2E_LOOPBACK_TOKEN;
  return browser.waitUntil(
    async () => {
      const response = await fetch(`${gatewayBaseUrl}/v1/channels/loopback-webhook/outbox/claim`, {
        method: "POST",
        headers: { Authorization: `Bearer ${token}` },
      });
      if (response.status === 204) return false;
      if (response.status !== 200) throw new Error(`loopback outbox failed: ${response.status}`);
      return response.json();
    },
    { timeout: 60_000, interval: 250, timeoutMsg: "Agent Run did not reach the loopback outbox" },
  );
}

describe.skip("legacy Host integration settings", () => {
  it("pairs Browser and persists the bundled Plugin/Connector lifecycle", async () => {
    await openHostSettings("browser");
    await clickWhenReady('[data-testid="host-domain-browser-pair"]');
    await waitForDisplayed('[data-testid="host-domain-browser-pairing-code"]');
    const pairingNonce = await $('[data-testid="host-domain-browser-pairing-code"]').getText();
    expect(pairingNonce.length).toBeGreaterThan(20);
    const extensionId = await pairRealChromeExtension(pairingNonce);
    expect(extensionId).toMatch(/^[a-p]{32}$/);
    await clickWhenReady('[data-testid="host-domain-refresh"]');
    await expect($('[data-testid="settings-browser-page"]')).toHaveText(
      expect.stringContaining("confirmed"),
    );

    await openHostSettings("plugins");
    await clickWhenReady('[data-testid="host-domain-install-sample-crm"]');
    if (!(await (await samplePluginRow()).getText()).includes("enabled")) {
      await clickWhenReady(
        '[data-testid="host-domain-plugin-sample-crm"] [data-component="switch-root"]',
      );
      await browser.waitUntil(
        async () => (await (await samplePluginRow()).getText()).includes("enabled"),
        {
          timeout: 20_000,
          timeoutMsg: "sample-crm did not enable",
        },
      );
    }

    await clickWhenReady('[data-testid="host-domain-open-plugin-ui-dashboard"]');
    await waitForDisplayed('[data-testid="host-domain-plugin-ui"]');
    const customUiFrame = await $('[data-testid="host-domain-plugin-ui-frame"]');
    await expect(customUiFrame).toHaveAttribute("sandbox", "allow-scripts");
    const customUiUrl = await customUiFrame.getAttribute("src");
    expect(customUiUrl).toContain("hachimi-plugin-ui");
    await browser.switchFrame(customUiFrame);
    await browser.waitUntil(
      async () => (await $("#fixture").getAttribute("data-loaded")) === "true",
      { timeout: 20_000, timeoutMsg: "sample-crm Custom UI did not resolve its read-only Asset" },
    );
    await expect($("#fixture")).toHaveText(expect.stringContaining("sample-crm-v1"));
    await expect($("#ipc")).toHaveText("Direct Tauri IPC denied by host");
    await expect($("#ipc")).toHaveAttribute("data-safe", "true");
    expect(await browser.execute(() => typeof window.__TAURI__ === "undefined")).toBe(true);
    const directIpcBoundary = await browser.execute(() => ({
      frozen: Object.isFrozen(window.__TAURI_INTERNALS__),
      ipcType: typeof window.__TAURI_INTERNALS__?.ipc,
      postMessageType: typeof window.__TAURI_INTERNALS__?.postMessage,
    }));
    expect(directIpcBoundary).toEqual({
      frozen: true,
      ipcType: "function",
      postMessageType: "function",
    });
    await browser.execute(() => {
      window.__hachimiPluginIpcProbe = "pending";
      window.__TAURI_INTERNALS__.invoke("get_bootstrap_state").then(
        () => {
          window.__hachimiPluginIpcProbe = "allowed";
        },
        (error) => {
          window.__hachimiPluginIpcProbe = `denied:${String(error)}`;
        },
      );
      setTimeout(() => {
        if (window.__hachimiPluginIpcProbe === "pending") {
          window.__hachimiPluginIpcProbe = "transport_timeout";
        }
      }, 1_000);
    });
    await browser.pause(1_200);
    const directIpcProbe = await browser.execute(() => window.__hachimiPluginIpcProbe);
    if (directIpcProbe === "allowed") {
      throw new Error(`Plugin Custom UI reached Tauri IPC: ${JSON.stringify(directIpcBoundary)}`);
    }
    expect(
      directIpcProbe === "transport_timeout" || String(directIpcProbe).startsWith("denied:"),
    ).toBe(true);
    await browser.switchToParentFrame();

    await clickWhenReady(
      '[data-testid="host-domain-plugin-sample-crm"] [data-component="switch-root"]',
    );
    await browser.waitUntil(
      async () => !(await (await samplePluginRow()).getText()).includes("enabled"),
      { timeout: 20_000, timeoutMsg: "sample-crm did not disable" },
    );
    const disabledFrame = await $('[data-testid="host-domain-plugin-ui-frame"]');
    await browser.execute(
      (frame, url) => frame.setAttribute("src", url),
      disabledFrame,
      customUiUrl,
    );
    await browser.switchFrame(disabledFrame);
    await browser.waitUntil(
      async () => (await $("body").getText()).includes("surface unavailable"),
      { timeout: 20_000, timeoutMsg: "Disabled Plugin Custom UI URL remained readable" },
    );
    await browser.switchToParentFrame();
    await clickWhenReady(
      '[data-testid="host-domain-plugin-sample-crm"] [data-component="switch-root"]',
    );
    await browser.waitUntil(
      async () => (await (await samplePluginRow()).getText()).includes("enabled"),
      { timeout: 20_000, timeoutMsg: "sample-crm did not re-enable" },
    );

    await openHostSettings("connected-apps");
    await $('[data-testid="host-domain-connector-name"]').setValue("Desktop E2E CRM");
    await $('[data-testid="host-domain-connector-secret"]').setValue("ephemeral-e2e-credential");
    await clickWhenReady('[data-testid="host-domain-create-sample-account"]');
    await browser.waitUntil(
      async () => {
        for (const row of await $$('[data-testid^="host-domain-connector-"]')) {
          if ((await row.getText()).includes("healthy")) {
            connectorTestId = await row.getAttribute("data-testid");
            return true;
          }
        }
        return false;
      },
      { timeout: 20_000, timeoutMsg: "sample-crm account did not become healthy" },
    );

    await restartApplication();
    await openHostSettings("plugins");
    await expect(await samplePluginRow()).toHaveText(expect.stringContaining("enabled"));
    await openHostSettings("connected-apps");
    await expect($('[data-testid="settings-connected-apps-page"]')).toHaveText(
      expect.stringContaining("Desktop E2E CRM"),
    );
  });

  it("reconciles a durable mock-poll ingress without a second Agent runtime", async () => {
    await openHostSettings("gateway");
    await clickWhenReady('[data-testid="host-domain-mock-poll"]');
    await waitForDisplayed('[data-testid="host-domain-notice"]');
    await expect($('[data-testid="host-domain-notice"]')).toHaveText(
      expect.stringContaining("mock-poll"),
    );

    await restartApplication();
    await openHostSettings("gateway");
    await clickWhenReady('[data-testid="host-domain-gateway-reconcile"]');
    await $('[data-testid="host-domain-notice"]').waitForDisplayed({ timeout: 20_000 });
    await expect($('[data-testid="host-domain-notice"]')).toHaveText(
      expect.stringContaining("reconciliation"),
    );
  });

  it("routes authenticated HTTP ingress through one Agent Run and delivers once after restart", async () => {
    await openHostSettings("gateway");
    await enableGatewayStartup();
    const messageId = `desktop-e2e-loopback-${Date.now()}`;
    const accepted = await sendLoopback(messageId);
    expect(accepted.receipt.status).toBe("accepted");
    const delivery = await claimLoopbackResult();
    expect(delivery.idempotencyKey).toBe(`channel-result:${messageId}`);
    expect(delivery.text.length).toBeGreaterThan(0);

    await restartApplication();
    await openHostSettings("gateway");
    await waitForGatewayEndpoint();
    const duplicate = await sendLoopback(messageId);
    expect(duplicate.receipt.status).toBe("duplicate");
    await browser.pause(750);
    const response = await fetch(`${gatewayBaseUrl}/v1/channels/loopback-webhook/outbox/claim`, {
      method: "POST",
      headers: { Authorization: `Bearer ${process.env.HACHIMI_DESKTOP_E2E_LOOPBACK_TOKEN}` },
    });
    expect(response.status).toBe(204);
    await openHostSettings("plugins");
    await clickWhenReady('[data-testid="host-domain-refresh"]');
    await browser.waitUntil(
      async () =>
        /hook_(run|tool)_after_ok/.test(await $('[data-testid="settings-plugins-page"]').getText()),
      { timeout: 20_000, timeoutMsg: "Agent Run did not execute the sample-crm Hook sidecar" },
    );
  });

  after(async () => {
    try {
      await openHostSettings("gateway");
      await disableGatewayStartup();
      if (connectorTestId) {
        await openHostSettings("connected-apps");
        const account = await $(`[data-testid="${connectorTestId}"]`);
        if (await account.isExisting()) {
          const revoke = await account.$("button");
          if (await revoke.isExisting()) await revoke.click();
        }
      }
      // The next Desktop E2E spec reuses this isolated data root. Do not
      // uninstall the bundled Plugin while a Hook sidecar may still hold its
      // executable on Windows; a locked best-effort removal can leave a
      // partial bundle and correctly trigger content-drift fail-closed.
      // The runner owns and removes the whole data root after the session.
    } catch {
      // The E2E data root is isolated. Preserve the original test result if a
      // closing WebDriver session prevents best-effort credential cleanup.
    }
    cleanupGatewayProcesses(process.env.HACHIMI_DESKTOP_E2E_APP);
    await waitForGatewayEndpointClosed();
  });
});

describe("Hachimi platform integrations", () => {
  it("exposes the five capability-driven Provider panels", async () => {
    await openHostSettings("integrations");
    for (const [providerId, label] of [
      ["dingtalk", "钉钉"],
      ["feishu", "飞书"],
      ["wecom_ai_bot", "企微 AI Bot"],
      ["wecom_app", "企微自建应用"],
      ["wechat_ilink", "微信 iLink / ClawBot"],
    ]) {
      await clickWhenReady(`button[role="tab"][aria-label="${label}"]`);
      await waitForDisplayed(`[data-testid="integration-provider-${providerId}"]`);
      await expect($(`[data-testid="integration-connect-${providerId}"]`)).toBeDisplayed();
    }
  });

  it("creates, restores, and removes a wecom_app account through the six-step wizard", async () => {
    await openHostSettings("integrations");
    await clickWhenReady('button[role="tab"][aria-label="企微自建应用"]');
    await waitForDisplayed('[data-testid="integration-provider-wecom_app"]');
    await clickWhenReady('[data-testid="integration-connect-wecom_app"]');
    await waitForDisplayed('.integration-wizard[data-step="1"]');

    await (await textFieldByLabel("账户名称")).setValue("Desktop E2E 企业微信");
    await clickWhenReady('[data-testid="integration-wizard-primary-action"]');
    await waitForDisplayed('.integration-wizard[data-step="2"]');
    await (await textFieldByLabel("Corp ID")).setValue("desktop-e2e-corp");
    await (await textFieldByLabel("Corp Secret")).setValue("desktop-e2e-secret");
    await (await textFieldByLabel("Agent ID")).setValue("1000002");
    await (await textFieldByLabel("Callback Token")).setValue("desktop-e2e-callback");
    await (
      await textFieldByLabel("Encoding AES Key")
    ).setValue("abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG");
    await (
      await textFieldByLabel("External HTTPS URL")
    ).setValue("https://127.0.0.1:42371/channels/wecom");
    await clickWhenReady('[data-testid="integration-wizard-primary-action"]');
    await waitForDisplayed('.integration-wizard[data-step="3"]');
    await clickWhenReady('[data-testid="integration-wizard-primary-action"]');
    await waitForDisplayed('.integration-wizard[data-step="4"]');
    await clickWhenReady('[data-testid="integration-wizard-primary-action"]');
    await waitForDisplayed('.integration-wizard[data-step="5"]');
    await clickWhenReady('[data-testid="integration-wizard-primary-action"]');
    await waitForDisplayed('.integration-wizard[data-step="6"]');
    await clickWhenReady('[data-testid="integration-wizard-primary-action"]');

    await waitForIntegrationAccount("wecom_app", "Desktop E2E 企业微信", true);

    await restartApplication();
    await openHostSettings("integrations");
    await clickWhenReady('button[role="tab"][aria-label="企微自建应用"]');
    await expect($('[data-testid="integration-provider-wecom_app"]')).toHaveText(
      expect.stringContaining("Desktop E2E 企业微信"),
    );

    const account = await $(
      '[data-testid="integration-provider-wecom_app"] [data-testid^="integration-account-"]',
    );
    const disconnect = await account.$('button[aria-label="断开连接"]');
    await disconnect.click();
    await clickWhenReady('[data-testid="integration-disconnect-confirm"]');
    await waitForIntegrationAccount("wecom_app", "Desktop E2E 企业微信", false);
  });

  after(async () => {
    cleanupGatewayProcesses(process.env.HACHIMI_DESKTOP_E2E_APP);
  });
});
