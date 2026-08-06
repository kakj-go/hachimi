import { spawn, spawnSync } from "node:child_process";
import { createConnection } from "node:net";
import { createServer } from "node:http";
import {
  createWriteStream,
  copyFileSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { createOfficeArtifact } from "./support/office-artifacts.mjs";
import { cleanupExecutableProcesses, terminateProcessTree } from "./support/processes.mjs";

async function allocateLoopbackPorts(count) {
  const reservations = [];
  try {
    for (let index = 0; index < count; index += 1) {
      const reservation = createServer();
      await new Promise((resolveListen, rejectListen) => {
        reservation.once("error", rejectListen);
        reservation.listen(0, "127.0.0.1", resolveListen);
      });
      reservations.push(reservation);
    }
    return reservations.map((reservation) => reservation.address().port);
  } finally {
    await Promise.all(
      reservations.map(
        (reservation) => new Promise((resolveClose) => reservation.close(resolveClose)),
      ),
    );
  }
}

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const corepackCli = join(
  dirname(process.execPath),
  "node_modules",
  "corepack",
  "dist",
  "corepack.js",
);
const temporaryRoot = mkdtempSync(join(tmpdir(), "hachimi-desktop-e2e-"));
const project = join(temporaryRoot, "project");
const data = join(temporaryRoot, "data");
const webviewData = join(data, "webview");
const attachment = join(temporaryRoot, "reference.txt");
const artifacts = resolve(root, "target/desktop-e2e-artifacts");
const buildTarget = resolve(root, "target/desktop-e2e-build");
const targetRoot = resolve(root, "target");
const driver =
  process.env.TAURI_DRIVER ?? resolve(root, "target/desktop-e2e-tools/bin/tauri-driver.exe");
const nativeDriver =
  process.env.MSEDGEDRIVER ?? resolve(root, "target/desktop-e2e-tools/msedgedriver.exe");
if (!existsSync(driver) || !existsSync(nativeDriver)) {
  throw new Error(
    `Desktop E2E drivers are missing. Set TAURI_DRIVER and MSEDGEDRIVER, or provision ${driver} and ${nativeDriver}.`,
  );
}
mkdirSync(project, { recursive: true });
mkdirSync(data, { recursive: true });
writeFileSync(join(data, ".hachimi-data-root"), "com.hachimi.desktop", "utf8");
const loopbackToken = "hachimi-desktop-e2e-loopback-token-00000001";
mkdirSync(join(data, "gateway"), { recursive: true });
writeFileSync(join(data, "gateway", "loopback.token"), loopbackToken, "utf8");
mkdirSync(webviewData, { recursive: true });
if (!artifacts.startsWith(`${targetRoot}${sep}`)) {
  throw new Error("Desktop E2E artifact path escaped the repository target directory.");
}
rmSync(artifacts, { recursive: true, force: true });
mkdirSync(artifacts, { recursive: true });
const officeStdioTemplates = join(artifacts, "office-stdio-templates");
mkdirSync(officeStdioTemplates, { recursive: true });
for (const extension of ["docx", "xlsx", "pptx", "pdf"]) {
  createOfficeArtifact(
    join(officeStdioTemplates, `template.${extension}`),
    extension,
    "Hachimi restricted Office E2E",
    `Validated ${extension} template`,
  );
  createOfficeArtifact(
    join(officeStdioTemplates, `modified.${extension}`),
    extension,
    "Hachimi restricted Office E2E revised",
    `Modified and revalidated ${extension} template`,
  );
}
const officeStdioServer = join(artifacts, "office-mcp-stdio.mjs");
copyFileSync(join(root, "scripts/desktop-e2e/support/office-mcp-stdio.mjs"), officeStdioServer);
writeFileSync(join(project, "README.md"), "# Desktop E2E fixture\n", "utf8");
writeFileSync(attachment, "Use the deterministic Desktop E2E workflow.\n", "utf8");

let mcpToolSchemaRevision = 1;
const mcpServer = createServer((request, response) => {
  const requestPath = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
  if (requestPath.includes("browser-")) {
    console.log(`[desktop-e2e-browser-fixture] ${request.method} ${request.url}`);
  }
  if (requestPath === "/browser-fixture" && request.method === "GET") {
    response.writeHead(200, {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
    }).end(`<!doctype html>
<html><head><title>Hachimi Browser Host E2E</title></head>
<body>
  <h1>Managed Browser scheduled fixture</h1>
  <p id="resource-status">resource pending</p>
  <input id="upload" type="file" />
  <a id="download" href="/browser-download.txt" download="hachimi-browser-e2e.txt">Download fixture</a>
  <script src="/browser-resource.js"></script>
</body></html>`);
    return;
  }
  if (requestPath === "/browser-resource.js" && request.method === "GET") {
    response
      .writeHead(200, {
        "content-type": "text/javascript; charset=utf-8",
        "cache-control": "no-store",
      })
      .end('document.querySelector("#resource-status").textContent = "resource loaded";');
    return;
  }
  if (requestPath === "/browser-download.txt" && request.method === "GET") {
    const body = "Hachimi managed Browser scheduled download fixture\n";
    response
      .writeHead(200, {
        "content-type": "text/plain",
        "content-disposition": "attachment; filename=hachimi-browser-e2e.txt",
        "content-length": Buffer.byteLength(body),
        connection: "close",
        "cache-control": "no-store",
      })
      .end(body);
    return;
  }
  if (requestPath === "/e2e/schema-v2" && request.method === "POST") {
    mcpToolSchemaRevision = 2;
    response.writeHead(204).end();
    return;
  }
  if (request.method === "DELETE") {
    response.writeHead(204).end();
    return;
  }
  const chunks = [];
  request.on("data", (chunk) => chunks.push(chunk));
  request.on("end", () => {
    const body = Buffer.concat(chunks).toString("utf8");
    if (!body.trim()) {
      response.writeHead(400, { "content-type": "text/plain" }).end("request body required");
      return;
    }
    let payload;
    try {
      payload = JSON.parse(body);
    } catch {
      response.writeHead(400, { "content-type": "text/plain" }).end("invalid JSON");
      return;
    }
    if (payload.method === "notifications/initialized") {
      response.writeHead(202).end();
      return;
    }
    let result;
    if (payload.method === "initialize") {
      result = {
        protocolVersion: "2025-06-18",
        capabilities: { tools: {} },
        serverInfo: { name: "Hachimi Desktop E2E MCP", version: "1.0.0" },
      };
    } else if (payload.method === "tools/list") {
      result = {
        tools: [
          {
            name: "echo",
            description: "Echo deterministic Desktop E2E input",
            inputSchema: {
              type: "object",
              properties: { message: { type: "string", description: "Message to echo" } },
              required: ["message"],
              additionalProperties: false,
            },
          },
          ...[
            [
              "create_document",
              "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            ],
            [
              "create_spreadsheet",
              "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            ],
            [
              "create_presentation",
              "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            ],
            ["create_pdf", "application/pdf"],
          ].map(([name, mediaType]) => ({
            name,
            description: `Create one deterministic ${mediaType} artifact for Desktop E2E`,
            inputSchema: {
              type: "object",
              properties: {
                title: { type: "string" },
                body: { type: "string" },
                ...(name === "create_document" && mcpToolSchemaRevision > 1
                  ? {
                      schemaRevision: {
                        type: "integer",
                        const: mcpToolSchemaRevision,
                      },
                    }
                  : {}),
              },
              required: ["title", "body"],
              additionalProperties: false,
            },
          })),
          {
            name: "inspect_artifact",
            description: "Read bounded metadata for a controlled Office artifact",
            inputSchema: {
              type: "object",
              properties: { artifactId: { type: "string" } },
              required: ["artifactId"],
              additionalProperties: false,
            },
          },
          {
            name: "modify_artifact",
            description: "Modify and revalidate a controlled Office artifact",
            inputSchema: {
              type: "object",
              properties: {
                artifactId: { type: "string" },
                body: { type: "string" },
              },
              required: ["artifactId", "body"],
              additionalProperties: false,
            },
          },
          {
            name: "diff_artifact",
            description: "Produce a bounded per-file Office artifact diff summary",
            inputSchema: {
              type: "object",
              properties: { artifactId: { type: "string" } },
              required: ["artifactId"],
              additionalProperties: false,
            },
          },
          {
            name: "export_artifact",
            description: "Export a controlled PDF artifact",
            inputSchema: {
              type: "object",
              properties: {
                artifactId: { type: "string" },
                format: { type: "string", enum: ["pdf"] },
              },
              required: ["artifactId", "format"],
              additionalProperties: false,
            },
          },
          {
            name: "send_artifact",
            description: "Deliver a deterministic artifact to an exact external target",
            inputSchema: {
              type: "object",
              properties: {
                artifactId: { type: "string" },
                target: { type: "string" },
              },
              required: ["artifactId", "target"],
              additionalProperties: false,
            },
          },
          {
            name: "preview_file_plan",
            description: "Preview an authorized file-organization plan without mutating files",
            inputSchema: {
              type: "object",
              properties: {
                root: { type: "string" },
                actions: { type: "array", items: { type: "string" } },
              },
              required: ["root", "actions"],
              additionalProperties: false,
            },
          },
        ],
      };
    } else if (payload.method === "tools/call") {
      const toolName = String(payload.params?.name ?? "");
      if (toolName === "echo") {
        result = {
          content: [{ type: "text", text: String(payload.params?.arguments?.message ?? "") }],
        };
      } else if (toolName.startsWith("create_")) {
        const body = String(payload.params?.arguments?.body ?? "Deterministic Office artifact");
        const extension = {
          create_document: "docx",
          create_spreadsheet: "xlsx",
          create_presentation: "pptx",
          create_pdf: "pdf",
        }[toolName];
        const artifactId = `desktop-e2e-${toolName}`;
        const artifactPath = join(artifacts, `${artifactId}.${extension}`);
        const validation = createOfficeArtifact(
          artifactPath,
          extension,
          String(payload.params?.arguments?.title ?? "Hachimi Desktop E2E"),
          body,
        );
        result = {
          content: [
            {
              type: "text",
              text: `Created and validated controlled artifact ${artifactId}`,
            },
          ],
          structuredContent: { artifactId, validated: true, validation },
          isError: false,
        };
      } else if (toolName === "inspect_artifact") {
        const artifactId = String(payload.params?.arguments?.artifactId ?? "");
        const extension = artifactId.endsWith("create_pdf") ? "pdf" : "docx";
        const artifactPath = join(artifacts, `${artifactId}.${extension}`);
        result = {
          content: [{ type: "text", text: `Inspected bounded metadata for ${artifactId}` }],
          structuredContent: {
            artifactId,
            exists: existsSync(artifactPath),
            extension,
            contentIncluded: false,
          },
          isError: !existsSync(artifactPath),
        };
      } else if (toolName === "modify_artifact") {
        const artifactId = String(payload.params?.arguments?.artifactId ?? "");
        const artifactPath = join(artifacts, `${artifactId}.docx`);
        const validation = createOfficeArtifact(
          artifactPath,
          "docx",
          "Hachimi Office E2E revised",
          String(payload.params?.arguments?.body ?? "Modified Office artifact"),
        );
        result = {
          content: [{ type: "text", text: `Modified and revalidated ${artifactId}` }],
          structuredContent: { artifactId, modified: true, validated: true, validation },
          isError: false,
        };
      } else if (toolName === "diff_artifact") {
        const artifactId = String(payload.params?.arguments?.artifactId ?? "");
        const diff = {
          artifactId,
          status: "modified",
          beforeRevision: "created",
          afterRevision: "revalidated",
          changedParts: ["word/document.xml"],
          contentIncluded: false,
        };
        writeFileSync(
          join(artifacts, "desktop-e2e-artifact-diff.json"),
          JSON.stringify(diff),
          "utf8",
        );
        result = {
          content: [{ type: "text", text: `Produced bounded diff summary for ${artifactId}` }],
          structuredContent: diff,
          isError: false,
        };
      } else if (toolName === "export_artifact") {
        const artifactId = String(payload.params?.arguments?.artifactId ?? "");
        const destination = join(artifacts, "desktop-e2e-exported.pdf");
        copyFileSync(join(artifacts, `${artifactId}.pdf`), destination);
        result = {
          content: [{ type: "text", text: `Exported controlled artifact ${artifactId}` }],
          structuredContent: {
            artifactId,
            format: "pdf",
            fileName: "desktop-e2e-exported.pdf",
          },
          isError: false,
        };
      } else if (toolName === "preview_file_plan") {
        const preview = {
          root: String(payload.params?.arguments?.root ?? "authorized-fixture"),
          actions: Array.isArray(payload.params?.arguments?.actions)
            ? payload.params.arguments.actions
            : [],
          inventory: ["incoming/report.docx", "incoming/report (1).docx"],
          conflictPolicy: "suffix",
          conflictExample: "organized/report (2).docx",
          authorizedRootBoundary: {
            normalizedRoot: "authorized-fixture",
            outsideRootRejected: true,
          },
          previewOnly: true,
        };
        const rollback = {
          version: 1,
          previewOnly: true,
          operations: preview.actions.map((action, index) => ({
            order: index,
            action,
            rollback: `undo:${action}`,
          })),
        };
        writeFileSync(
          join(artifacts, "desktop-e2e-file-plan.json"),
          JSON.stringify(preview),
          "utf8",
        );
        writeFileSync(
          join(artifacts, "desktop-e2e-file-rollback.json"),
          JSON.stringify(rollback),
          "utf8",
        );
        result = {
          content: [{ type: "text", text: "File organization plan previewed without mutation" }],
          structuredContent: preview,
          isError: false,
        };
      } else if (toolName === "send_artifact") {
        const receipt = {
          artifactId: String(payload.params?.arguments?.artifactId ?? ""),
          target: String(payload.params?.arguments?.target ?? ""),
          delivered: true,
        };
        writeFileSync(
          join(artifacts, "desktop-e2e-office-delivery.json"),
          JSON.stringify(receipt),
          "utf8",
        );
        result = {
          content: [{ type: "text", text: "Deterministic external delivery completed" }],
          structuredContent: receipt,
          isError: false,
        };
      } else {
        result = {
          content: [{ type: "text", text: "Unknown Desktop E2E MCP tool" }],
          isError: true,
        };
      }
    } else {
      response.writeHead(200, { "content-type": "application/json" }).end(
        JSON.stringify({
          jsonrpc: "2.0",
          id: payload.id,
          error: { code: -32601, message: "method not found" },
        }),
      );
      return;
    }
    response
      .writeHead(200, {
        "content-type": "application/json",
        "mcp-session-id": "hachimi-desktop-e2e-session",
      })
      .end(JSON.stringify({ jsonrpc: "2.0", id: payload.id, result }));
  });
});
await new Promise((resolveListen) => mcpServer.listen(0, "127.0.0.1", resolveListen));
const mcpAddress = mcpServer.address();
if (!mcpAddress || typeof mcpAddress === "string")
  throw new Error("Desktop E2E MCP failed to bind");
const mcpUrl = `http://127.0.0.1:${mcpAddress.port}/mcp`;
const browserFixtureUrl = `http://127.0.0.1:${mcpAddress.port}/browser-fixture`;
const browserFixtureOrigin = `http://127.0.0.1:${mcpAddress.port}`;
const [gatewayPort, gatewayWakePort] = await allocateLoopbackPorts(2);

function checked(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    windowsHide: true,
    ...options,
  });
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}

async function checkedAsync(command, args, options = {}) {
  const child = spawn(command, args, {
    cwd: root,
    stdio: "inherit",
    windowsHide: true,
    ...options,
  });
  activeChildren.add(child);
  try {
    const code = await new Promise((resolveExit, rejectExit) => {
      child.once("error", rejectExit);
      child.once("exit", resolveExit);
    });
    if (code !== 0) {
      throw new Error(`${command} ${args.join(" ")} failed with exit code ${code}`);
    }
  } finally {
    activeChildren.delete(child);
  }
}

const activeChildren = new Set();

const testEnvironment = {
  ...process.env,
  CARGO_NET_OFFLINE: "true",
  CARGO_TARGET_DIR: buildTarget,
  TAURI_CONFIG: JSON.stringify({ build: { devUrl: null } }),
  HACHIMI_DATA_DIR: data,
  HACHIMI_DESKTOP_E2E_WEBVIEW_DATA: webviewData,
  HACHIMI_DESKTOP_E2E_PROJECT_PATH: project,
  HACHIMI_DESKTOP_E2E_ATTACHMENT_PATH: attachment,
  HACHIMI_DESKTOP_E2E_SANDBOX: "deterministic",
  HACHIMI_DESKTOP_E2E_PROVIDER: "deterministic",
  HACHIMI_DESKTOP_E2E_ARTIFACTS: artifacts,
  HACHIMI_DESKTOP_E2E_LOOPBACK_TOKEN: loopbackToken,
  HACHIMI_DESKTOP_E2E_GATEWAY_PORT: String(gatewayPort),
  HACHIMI_DESKTOP_E2E_GATEWAY_WAKE_PORT: String(gatewayWakePort),
  HACHIMI_DESKTOP_E2E_MCP_URL: mcpUrl,
  HACHIMI_DESKTOP_E2E_MCP_STDIO_COMMAND: process.execPath,
  HACHIMI_DESKTOP_E2E_MCP_STDIO_ARGS: officeStdioServer,
  HACHIMI_DESKTOP_E2E_MCP_STDIO_CWD: artifacts,
  HACHIMI_DESKTOP_E2E_BROWSER_URL: browserFixtureUrl,
  HACHIMI_DESKTOP_E2E_BROWSER_ORIGIN: browserFixtureOrigin,
  HACHIMI_MANAGED_CHROMIUM: join(root, "apps/desktop/src-tauri/managed-chromium/chrome.exe"),
};

checked("node", ["scripts/prepare-workspace-worker.mjs", "dev"], {
  env: testEnvironment,
});
if (process.env.HACHIMI_DESKTOP_E2E_REAL_SANDBOX === "1") {
  delete testEnvironment.HACHIMI_DESKTOP_E2E_SANDBOX;
  const e2eDebugRoot = join(buildTarget, "debug");
  const setup = join(e2eDebugRoot, "hachimi-sandbox-setup.exe");
  const launcher = join(e2eDebugRoot, "hachimi-sandbox-launcher.exe");
  const marker = join(data, "sandbox/windows/setup.json");
  checked(setup, ["--marker", marker, "--launcher", launcher], { env: testEnvironment });
}
checked(process.execPath, [corepackCli, "pnpm", "--dir", "apps/desktop/web", "build"], {
  env: testEnvironment,
});
const desktopPdb = join(buildTarget, "debug", "deps", "hachimi_desktop.pdb");
if (!desktopPdb.startsWith(`${buildTarget}${sep}`)) {
  throw new Error("Desktop E2E PDB path escaped the dedicated build directory.");
}
// MSVC can retain exhausted type-server state when repeatedly relinking this
// large debug binary. The PDB is a disposable E2E build artifact; recreating
// just this file avoids LNK1318 without cleaning any source or shared target.
rmSync(desktopPdb, { force: true });
checked(
  process.execPath,
  [
    "scripts/run-with-rust.mjs",
    "cargo",
    "build",
    "--offline",
    "-p",
    "hachimi-desktop",
    "--features",
    "desktop-e2e",
  ],
  { env: testEnvironment },
);

testEnvironment.HACHIMI_DESKTOP_E2E_APP = resolve(buildTarget, "debug/hachimi-desktop.exe");
const consoleStopFile = join(artifacts, "console-window-monitor.stop");
const consoleReportFile = join(artifacts, "console-window-monitor.json");
let consoleMonitorProcess;
let consoleMonitorExit;
if (process.platform === "win32") {
  consoleMonitorProcess = spawn(
    "powershell.exe",
    [
      "-NoLogo",
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      join(root, "scripts/desktop-e2e/support/console-window-monitor.ps1"),
      "-ApplicationPath",
      testEnvironment.HACHIMI_DESKTOP_E2E_APP,
      "-StopFile",
      consoleStopFile,
      "-ReportFile",
      consoleReportFile,
    ],
    {
      cwd: root,
      env: testEnvironment,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    },
  );
  consoleMonitorProcess.stdout.pipe(process.stdout);
  consoleMonitorProcess.stderr.pipe(process.stderr);
  consoleMonitorExit = new Promise((resolveExit) => {
    consoleMonitorProcess.once("exit", resolveExit);
  });
}
const driverProcess = spawn(driver, ["--native-driver", nativeDriver], {
  cwd: root,
  env: testEnvironment,
  stdio: ["ignore", "pipe", "pipe"],
  windowsHide: true,
});
driverProcess.stdout.pipe(process.stdout);
driverProcess.stderr.pipe(process.stderr);
const driverLog = createWriteStream(join(artifacts, "tauri-driver.log"), { flags: "w" });
driverProcess.stdout.pipe(driverLog);
driverProcess.stderr.pipe(driverLog);

async function waitForPort(port, description) {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    const connected = await new Promise((resolveConnection) => {
      const socket = createConnection({ host: "127.0.0.1", port });
      socket.once("connect", () => {
        socket.destroy();
        resolveConnection(true);
      });
      socket.once("error", () => resolveConnection(false));
    });
    if (connected) return;
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
  }
  throw new Error(`${description} did not listen on port ${port} within 15 seconds`);
}

let succeeded = false;
let consoleWindowFailure;
try {
  await waitForPort(4444, "tauri-driver");
  await checkedAsync(
    process.execPath,
    ["node_modules/@wdio/cli/bin/wdio.js", "run", "scripts/desktop-e2e/wdio.conf.mjs"],
    { env: testEnvironment },
  );
  succeeded = true;
} finally {
  for (const child of activeChildren) terminateProcessTree(child.pid);
  terminateProcessTree(driverProcess.pid);
  cleanupExecutableProcesses(testEnvironment.HACHIMI_DESKTOP_E2E_APP);
  if (consoleMonitorProcess) {
    writeFileSync(consoleStopFile, "stop", "utf8");
    await Promise.race([
      consoleMonitorExit,
      new Promise((resolveDelay) => setTimeout(resolveDelay, 5_000)),
    ]);
    if (consoleMonitorProcess.exitCode == null) terminateProcessTree(consoleMonitorProcess.pid);
    if (existsSync(consoleReportFile)) {
      const report = JSON.parse(readFileSync(consoleReportFile, "utf8"));
      if (Array.isArray(report.findings) && report.findings.length > 0) {
        succeeded = false;
        consoleWindowFailure = `Desktop E2E observed ${report.findings.length} descendant ConsoleWindowClass window(s)`;
      }
    } else if (succeeded) {
      succeeded = false;
      consoleWindowFailure = "Desktop E2E console-window monitor did not produce a report";
    }
  }
  driverLog.end();
  await new Promise((resolveClose) => mcpServer.close(resolveClose));
  if (succeeded && process.env.HACHIMI_KEEP_DESKTOP_E2E !== "1") {
    rmSync(temporaryRoot, { recursive: true, force: true });
  } else {
    console.error(`Desktop E2E fixture retained at ${temporaryRoot}`);
  }
}
if (consoleWindowFailure) throw new Error(consoleWindowFailure);
