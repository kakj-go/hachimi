import { spawn, spawnSync } from "node:child_process";
import { createServer as createHttpServer } from "node:http";
import { createServer as createNetServer } from "node:net";
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const seconds = Number.parseInt(process.env.HACHIMI_STRESS_PHASE_SECONDS ?? "300", 10);
const chrome = resolve(
  process.env.HACHIMI_MANAGED_CHROMIUM ?? "apps/desktop/src-tauri/managed-chromium/chrome.exe",
);
const root = mkdtempSync(join(tmpdir(), "hachimi-browser-stress-"));
const profile = join(root, "profile");
const downloads = join(root, "downloads");
const upload = join(root, "upload.txt");
mkdirSync(profile, { recursive: true });
mkdirSync(downloads, { recursive: true });
mkdirSync(join(profile, "Default"), { recursive: true });
writeFileSync(
  join(profile, "Default", "Preferences"),
  JSON.stringify({
    download: {
      default_directory: downloads,
      directory_upgrade: true,
      prompt_for_download: false,
    },
    profile: { default_content_setting_values: { automatic_downloads: 1 } },
  }),
  "utf8",
);
writeFileSync(upload, "Hachimi Browser stress upload fixture\n", "utf8");

let uploadCount = 0;
let downloadRequestCount = 0;
const downloadBody = Buffer.from("Hachimi Browser stress download fixture\n", "utf8");
const server = createHttpServer((request, response) => {
  if (request.url === "/fixture") {
    response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    response.end(`<!doctype html><html><body>
      <input id="text" value=""><button id="increment">Increment</button>
      <output id="counter">0</output>
      <form id="upload-form" action="/upload" method="post" enctype="multipart/form-data">
        <input id="upload" type="file" name="fixture">
      </form>
      <a id="download" download="browser-stress-download.txt" href="/download">Download</a>
      <script>
        increment.onclick = () => counter.textContent = String(Number(counter.textContent) + 1);
      </script>
    </body></html>`);
    return;
  }
  if (request.url === "/upload" && request.method === "POST") {
    request.on("data", () => {});
    request.on("end", () => {
      uploadCount += 1;
      response.writeHead(204).end();
    });
    return;
  }
  if (request.url === "/download") {
    downloadRequestCount += 1;
    response
      .writeHead(200, {
        "content-type": "text/plain",
        "content-disposition": 'attachment; filename="browser-stress-download.txt"',
        "content-length": downloadBody.byteLength,
      })
      .end(downloadBody);
    return;
  }
  response.writeHead(404).end();
});
await new Promise((resolveListen) => server.listen(0, "127.0.0.1", resolveListen));
const address = server.address();
if (!address || typeof address === "string") throw new Error("Browser stress fixture did not bind");

const portReservation = createNetServer();
await new Promise((resolveListen) => portReservation.listen(0, "127.0.0.1", resolveListen));
const reservedAddress = portReservation.address();
if (!reservedAddress || typeof reservedAddress === "string") {
  throw new Error("Browser stress debugging port reservation failed");
}
const debuggingPort = reservedAddress.port;
await new Promise((resolveClose) => portReservation.close(resolveClose));
const child = spawn(
  chrome,
  [
    "--headless=new",
    "--disable-gpu",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-default-apps",
    "--disable-features=DownloadBubble",
    "--disable-sync",
    "--no-first-run",
    `--download-default-directory=${downloads}`,
    `--remote-debugging-port=${debuggingPort}`,
    `--user-data-dir=${profile}`,
    "about:blank",
  ],
  { stdio: "ignore", windowsHide: true },
);

async function waitForJson(url, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) return response.json();
    } catch {
      // Chromium is still starting.
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
  }
  throw new Error(`Managed Chromium endpoint did not become ready: ${url}`);
}

async function removeWithRetry(path, timeoutMs = 5_000) {
  const deadline = Date.now() + timeoutMs;
  while (true) {
    try {
      rmSync(path, { force: true, recursive: true });
      return;
    } catch (error) {
      if (
        !error ||
        !["EPERM", "EBUSY", "ENOTEMPTY"].includes(error.code) ||
        Date.now() >= deadline
      ) {
        throw error;
      }
      await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
    }
  }
}

class CdpSession {
  constructor(url) {
    this.url = url;
    this.nextId = 1;
    this.pending = new Map();
    this.events = [];
  }

  async connect() {
    this.socket = new WebSocket(this.url);
    this.socket.onmessage = (event) => {
      const message = JSON.parse(String(event.data));
      if (!message.id) {
        this.events.push(message);
        return;
      }
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.error) pending.reject(new Error(message.error.message));
      else pending.resolve(message.result ?? {});
    };
    await new Promise((resolveOpen, rejectOpen) => {
      this.socket.onopen = resolveOpen;
      this.socket.onerror = () => rejectOpen(new Error("CDP WebSocket failed to connect"));
    });
  }

  call(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolveCall, rejectCall) => {
      this.pending.set(id, { resolve: resolveCall, reject: rejectCall });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  close() {
    this.socket?.close();
  }
}

let pageId;
let session;
let browserSession;
try {
  const version = await waitForJson(`http://127.0.0.1:${debuggingPort}/json/version`);
  if (!version.webSocketDebuggerUrl) {
    throw new Error("Managed Chromium did not expose a browser CDP target");
  }
  browserSession = new CdpSession(version.webSocketDebuggerUrl);
  await browserSession.connect();
  await browserSession.call("Browser.setDownloadBehavior", {
    behavior: "allow",
    downloadPath: downloads,
    eventsEnabled: true,
  });
  const page = await fetch(
    `http://127.0.0.1:${debuggingPort}/json/new?${encodeURIComponent(`http://127.0.0.1:${address.port}/fixture`)}`,
    { method: "PUT" },
  ).then((response) => response.json());
  pageId = page.id;
  session = new CdpSession(page.webSocketDebuggerUrl);
  await session.connect();
  await session.call("Page.enable");
  await session.call("DOM.enable");
  // Headless Chromium versions differ on whether the Browser-domain policy is
  // inherited by a page target. Keep the page-domain policy as a compatibility
  // fallback; unsupported older targets can safely ignore it.
  try {
    await session.call("Page.setDownloadBehavior", { behavior: "allow", downloadPath: downloads });
  } catch {
    // Browser.setDownloadBehavior above is authoritative on newer Chromium.
  }
  await session.call("Page.navigate", {
    url: `http://127.0.0.1:${address.port}/fixture`,
  });
  const deadline = Date.now() + seconds * 1_000;
  let iterations = 0;
  while (Date.now() < deadline) {
    const ready = await session.call("Runtime.evaluate", {
      expression: "document.readyState",
      returnByValue: true,
    });
    if (ready.result?.value !== "complete") {
      await new Promise((resolveDelay) => setTimeout(resolveDelay, 25));
      continue;
    }
    const observed = await session.call("Runtime.evaluate", {
      expression: "Number(document.querySelector('#counter').textContent)",
      returnByValue: true,
    });
    await session.call("Runtime.evaluate", {
      expression: `document.querySelector('#text').value = ${JSON.stringify(`iteration-${iterations}`)}; document.querySelector('#increment').click()`,
    });
    const acted = await session.call("Runtime.evaluate", {
      expression: "Number(document.querySelector('#counter').textContent)",
      returnByValue: true,
    });
    if (acted.result?.value !== observed.result?.value + 1) {
      throw new Error(
        `stale_observation_fence_failed: observed=${JSON.stringify(observed)} acted=${JSON.stringify(acted)}`,
      );
    }
    const screenshot = await session.call("Page.captureScreenshot", { format: "png" });
    if (!screenshot.data || screenshot.data.length < 100) throw new Error("blank_browser_frame");

    if (iterations % 25 === 0) {
      const documentNode = await session.call("DOM.getDocument", { depth: 2 });
      const fileInput = await session.call("DOM.querySelector", {
        nodeId: documentNode.root.nodeId,
        selector: "#upload",
      });
      await session.call("DOM.setFileInputFiles", { nodeId: fileInput.nodeId, files: [upload] });
      const beforeUpload = uploadCount;
      await session.call("Runtime.evaluate", {
        expression: "document.querySelector('#upload-form').requestSubmit()",
      });
      const uploadDeadline = Date.now() + 5_000;
      while (uploadCount === beforeUpload && Date.now() < uploadDeadline) {
        await new Promise((resolveDelay) => setTimeout(resolveDelay, 25));
      }
      if (uploadCount === beforeUpload) throw new Error("browser_upload_timeout");
      await session.call("Page.navigate", {
        url: `http://127.0.0.1:${address.port}/fixture`,
      });
      while (
        (
          await session.call("Runtime.evaluate", {
            expression: "document.readyState",
            returnByValue: true,
          })
        ).result?.value !== "complete"
      ) {
        await new Promise((resolveDelay) => setTimeout(resolveDelay, 25));
      }
      // Navigating directly to the attachment URL is deterministic in
      // headless Chromium; synthetic DOM clicks are not treated as download
      // user gestures by all managed Chromium revisions.
      await session.call("Page.navigate", {
        url: `http://127.0.0.1:${address.port}/download`,
        transitionType: "typed",
      });
      const downloadDeadline = Date.now() + 10_000;
      let downloaded;
      while (!downloaded && Date.now() < downloadDeadline) {
        const progress = browserSession.events.filter(
          (event) => event.method === "Browser.downloadProgress",
        );
        if (progress.some((event) => event.params?.state === "canceled")) {
          throw new Error("browser_download_canceled");
        }
        const candidates = readdirSync(downloads).filter((file) => !file.endsWith(".crdownload"));
        if (progress.some((event) => event.params?.state === "completed")) {
          [downloaded] = candidates;
        }
        await new Promise((resolveDelay) => setTimeout(resolveDelay, 25));
      }
      if (!downloaded) {
        const events = browserSession.events.filter((event) =>
          String(event.method ?? "").startsWith("Browser.download"),
        );
        throw new Error(
          `browser_download_timeout: requests=${downloadRequestCount} events=${JSON.stringify(events)}`,
        );
      }
      const contents = readFileSync(join(downloads, downloaded), "utf8");
      if (!contents.includes("Hachimi Browser stress download fixture")) {
        throw new Error("browser_download_corrupt");
      }
      await removeWithRetry(join(downloads, downloaded));
      browserSession.events.length = 0;
    }
    if (iterations > 0 && iterations % 50 === 0) {
      session.close();
      session = new CdpSession(page.webSocketDebuggerUrl);
      await session.connect();
      await session.call("Page.enable");
      await session.call("DOM.enable");
    }
    iterations += 1;
  }
  if (iterations === 0) throw new Error("Managed Chromium stress completed no iterations");
  process.stdout.write(`browser_real_stress_iterations=${iterations}\n`);
} finally {
  session?.close();
  browserSession?.close();
  if (pageId) {
    await fetch(`http://127.0.0.1:${debuggingPort}/json/close/${pageId}`).catch(() => {});
  }
  child.kill();
  await Promise.race([
    new Promise((resolveExit) => child.once("exit", resolveExit)),
    new Promise((resolveDelay) => setTimeout(resolveDelay, 2_000)),
  ]);
  if (child.exitCode == null && process.platform === "win32") {
    spawnSync("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
  }
  await new Promise((resolveClose) => server.close(resolveClose));
  await removeWithRetry(root);
}
