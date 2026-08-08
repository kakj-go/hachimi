import { join } from "node:path";

import { _setGlobal } from "@wdio/globals";
import { remote } from "webdriverio";

import { cleanupExecutableProcesses } from "./processes.mjs";

let restartSequence = 0;

export async function switchToWorkbench() {
  await browser.waitUntil(
    async () => {
      const handles = await browser.getWindowHandles();
      for (const handle of handles) {
        try {
          await browser.switchToWindow(handle);
          const [title, url] = await Promise.all([browser.getTitle(), browser.getUrl()]);
          if (title.includes("Hachimi Workbench") || url.includes("/workbench.html")) return true;
        } catch {
          // A startup WebView may disappear while the native app creates the
          // Workbench. The next poll only considers current handles.
        }
      }
      return false;
    },
    {
      timeout: 45_000,
      interval: 100,
      timeoutMsg: "Hachimi Workbench window was not ready",
    },
  );
}

export async function switchToPet() {
  await browser.waitUntil(
    async () => {
      const handles = await browser.getWindowHandles();
      for (const handle of handles) {
        try {
          await browser.switchToWindow(handle);
          const [title, url] = await Promise.all([browser.getTitle(), browser.getUrl()]);
          if (title.includes("Hachimi Pet") || url.includes("/pet.html")) return true;
        } catch {
          // The Workbench can hide while the native shell restores the Pet.
        }
      }
      return false;
    },
    { timeout: 20_000, interval: 100, timeoutMsg: "Hachimi Pet window was not ready" },
  );
}

export async function restartApplication() {
  const application = process.env.HACHIMI_DESKTOP_E2E_APP;
  const webviewData = process.env.HACHIMI_DESKTOP_E2E_WEBVIEW_DATA;
  if (!application || !webviewData) throw new Error("Desktop E2E restart paths are unavailable");
  restartSequence += 1;
  // Let tauri-driver release its current Edge session before terminating any
  // surviving application process. reloadSession() cannot do this reliably:
  // once the native process exits, its follow-up DELETE targets an invalid
  // Edge session and tauri-driver may retain the stale session internally.
  try {
    await browser.deleteSession();
  } catch {
    // The native process may have already exited. Exact-path cleanup below is
    // still required before asking tauri-driver for a replacement session.
  }
  cleanupExecutableProcesses(application);
  const replacement = await remote({
    hostname: "127.0.0.1",
    port: 4444,
    path: "/",
    logLevel: "warn",
    connectionRetryCount: 0,
    capabilities: {
      "tauri:options": {
        application,
        webviewOptions: {
          // The application database remains under HACHIMI_DATA_DIR. A new
          // browser-only profile avoids WebView2's short-lived Preferences lock.
          userDataFolder: join(webviewData, `restart-${restartSequence}`),
        },
      },
    },
  });
  _setGlobal("browser", replacement);
  _setGlobal("driver", replacement);
  _setGlobal("$", (selector) => replacement.$(selector));
  _setGlobal("$$", (selector) => replacement.$$(selector));
  await browser.waitUntil(async () => (await browser.getWindowHandles()).length > 0, {
    timeout: 30_000,
    interval: 100,
    timeoutMsg: "Restarted Hachimi process did not expose a WebDriver window",
  });
}
