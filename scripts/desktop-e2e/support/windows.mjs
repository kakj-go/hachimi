import { join } from "node:path";

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
  await browser.reloadSession({
    "tauri:options": {
      application,
      webviewOptions: {
        // The application database remains under HACHIMI_DATA_DIR. A new
        // browser-only profile avoids WebView2's short-lived Preferences lock.
        userDataFolder: join(webviewData, `restart-${restartSequence}`),
      },
    },
  });
  // tauri-driver can leave the old native process alive after reloadSession.
  // The application path is unique to this E2E build, so retain only the
  // newest exact executable instance and never match a user's installed app.
  cleanupExecutableProcesses(application, { keepNewest: 1 });
}
