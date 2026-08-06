import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

import { cleanupExecutableProcesses } from "./support/processes.mjs";

const artifacts =
  process.env.HACHIMI_DESKTOP_E2E_ARTIFACTS ?? resolve("target/desktop-e2e-artifacts");
mkdirSync(artifacts, { recursive: true });
const requestedSpec = process.env.HACHIMI_DESKTOP_E2E_SPEC;

export const config = {
  runner: "local",
  hostname: "127.0.0.1",
  port: 4444,
  path: "/",
  specs: requestedSpec
    ? [resolve(requestedSpec)]
    : [
        resolve("scripts/desktop-e2e/specs/workbench-core.e2e.mjs"),
        resolve("scripts/desktop-e2e/specs/agent-tools.e2e.mjs"),
        resolve("scripts/desktop-e2e/specs/extensions-settings.e2e.mjs"),
        resolve("scripts/desktop-e2e/specs/host-integrations.e2e.mjs"),
        resolve("scripts/desktop-e2e/specs/task-center.e2e.mjs"),
      ],
  maxInstances: 1,
  capabilities: [
    {
      maxInstances: 1,
      "tauri:options": {
        application: process.env.HACHIMI_DESKTOP_E2E_APP,
        webviewOptions: {
          userDataFolder: process.env.HACHIMI_DESKTOP_E2E_WEBVIEW_DATA,
        },
      },
    },
  ],
  logLevel: "warn",
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: {
    // The release lifecycle tests intentionally cross multiple fresh
    // checkout-bound Host processes, application restarts and ConPTY/MCP
    // boundaries. A single assertion keeps its own much smaller timeout; this
    // ceiling only prevents Mocha from aborting a progressing end-to-end flow.
    timeout: 600_000,
    grep: process.env.HACHIMI_DESKTOP_E2E_GREP || undefined,
  },
  connectionRetryCount: 0,
  afterTest: async (_test, _context, result) => {
    if (!result.passed) {
      const safeName = `failure-${Date.now()}.png`;
      try {
        await browser.saveScreenshot(resolve(artifacts, safeName));
      } catch {
        // Preserve the original failure when a restart already invalidated the
        // WebDriver session and no screenshot can be captured.
      }
    }
  },
  afterSession: () => {
    cleanupExecutableProcesses(process.env.HACHIMI_DESKTOP_E2E_APP);
  },
};
