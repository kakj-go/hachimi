import { spawn, spawnSync } from "node:child_process";
import { createServer as createHttpServer } from "node:http";
import { createServer as createNetServer } from "node:net";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const chrome = resolve(
  process.env.HACHIMI_MANAGED_CHROMIUM ?? "apps/desktop/src-tauri/managed-chromium/chrome.exe",
);
const root = mkdtempSync(join(tmpdir(), "hachimi-browser-host-stress-"));
const profile = join(root, "profile");
const fixture = createHttpServer((request, response) => {
  if (request.url !== "/fixture") {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
  response.end(`<!doctype html><html><head><title>Hachimi Browser Host fixture</title></head>
    <body><input id="text"><button id="increment">Increment</button><output id="counter">0</output>
    <div style="height:2000px">scroll fixture</div><script>
    increment.onclick=()=>counter.textContent=String(Number(counter.textContent)+1);
    </script></body></html>`);
});
await new Promise((resolveListen) => fixture.listen(0, "127.0.0.1", resolveListen));
const fixtureAddress = fixture.address();
if (!fixtureAddress || typeof fixtureAddress === "string") throw new Error("fixture bind failed");
const fixtureUrl = `http://127.0.0.1:${fixtureAddress.port}/fixture`;

const reservation = createNetServer();
await new Promise((resolveListen) => reservation.listen(0, "127.0.0.1", resolveListen));
const reserved = reservation.address();
if (!reserved || typeof reserved === "string") throw new Error("CDP port reservation failed");
await new Promise((resolveClose) => reservation.close(resolveClose));
const debuggingPort = reserved.port;
const child = spawn(
  chrome,
  [
    "--headless=new",
    "--disable-gpu",
    "--disable-background-networking",
    "--no-first-run",
    `--remote-debugging-port=${debuggingPort}`,
    `--user-data-dir=${profile}`,
    fixtureUrl,
  ],
  { stdio: "ignore", windowsHide: true },
);

async function createFixturePage() {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    try {
      const version = await fetch(`http://127.0.0.1:${debuggingPort}/json/version`);
      if (version.ok) {
        const page = await fetch(
          `http://127.0.0.1:${debuggingPort}/json/new?${encodeURIComponent(fixtureUrl)}`,
          { method: "PUT" },
        ).then((response) => response.json());
        if (page.webSocketDebuggerUrl) return page;
      }
    } catch {
      // Chromium is still starting.
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
  }
  throw new Error("managed Chromium did not expose a page target");
}

try {
  const page = await createFixturePage();
  await new Promise((resolveDelay) => setTimeout(resolveDelay, 750));
  const result = spawnSync(
    process.execPath,
    [
      "scripts/run-with-rust.mjs",
      "cargo",
      "test",
      "-p",
      "hachimi-browser",
      "--test",
      "real_host_stress",
      "real_managed_chromium_runs_through_browser_host_api",
      "--",
      "--ignored",
      "--nocapture",
      "--test-threads=1",
    ],
    {
      cwd: resolve("."),
      env: {
        ...process.env,
        HACHIMI_BROWSER_CDP_WS_URL: page.webSocketDebuggerUrl,
        HACHIMI_BROWSER_FIXTURE_URL: fixtureUrl,
      },
      stdio: "inherit",
      windowsHide: true,
    },
  );
  if (result.status !== 0) throw new Error(`BrowserHost stress failed: ${result.status}`);
} finally {
  child.kill();
  if (child.exitCode == null && process.platform === "win32") {
    spawnSync("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
  }
  await new Promise((resolveClose) => fixture.close(resolveClose));
  rmSync(root, { recursive: true, force: true });
}
