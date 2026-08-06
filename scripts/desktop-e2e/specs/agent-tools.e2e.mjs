import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { createServer } from "node:http";
import { dirname, join } from "node:path";

import { clickWhenReady, isDisplayed, waitForDisplayed } from "../support/interactions.mjs";
import { switchToWorkbench } from "../support/windows.mjs";

/* global document */

function git(project, ...args) {
  const result = spawnSync("git", args, { cwd: project, encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`git ${args.join(" ")} failed: ${result.stderr || result.stdout}`);
  }
  return result.stdout.trim();
}

function ensureGitRemoteFixture() {
  const project = process.env.HACHIMI_DESKTOP_E2E_PROJECT_PATH;
  if (!project) throw new Error("HACHIMI_DESKTOP_E2E_PROJECT_PATH is missing");
  const inside = spawnSync("git", ["rev-parse", "--is-inside-work-tree"], {
    cwd: project,
    encoding: "utf8",
  });
  if (inside.status !== 0) {
    git(project, "init", "--initial-branch=main");
    git(project, "config", "user.name", "Hachimi Desktop E2E");
    git(project, "config", "user.email", "desktop-e2e@hachimi.invalid");
    git(project, "add", "README.md");
    git(project, "commit", "-m", "Desktop E2E fixture");
  }
  const remote = spawnSync("git", ["remote", "get-url", "origin"], {
    cwd: project,
    encoding: "utf8",
  });
  if (remote.status === 0) {
    git(project, "remote", "set-url", "origin", "https://github.com/hachimi/desktop-e2e.git");
  } else {
    git(project, "remote", "add", "origin", "https://github.com/hachimi/desktop-e2e.git");
  }
}

function setRemote(project, name, url) {
  const remote = spawnSync("git", ["remote", "get-url", name], {
    cwd: project,
    encoding: "utf8",
  });
  git(project, "remote", remote.status === 0 ? "set-url" : "add", name, url);
}

function responseJson(response, status, value) {
  response.writeHead(status, { "content-type": "application/json" });
  response.end(JSON.stringify(value));
}

async function requestJson(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  if (chunks.length === 0) return {};
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

function forgeRecord(number, oid, overrides = {}) {
  return {
    number,
    title: `Desktop E2E change ${number}`,
    body: "initial body",
    head: { ref: `feature-${number}`, sha: oid },
    base: { ref: "main" },
    state: "open",
    merged: false,
    html_url: `http://127.0.0.1/owner/repo/pulls/${number}`,
    updated_at: `revision-${number}-1`,
    ...overrides,
  };
}

async function startForgeFixture(oid) {
  const state = {
    changes: new Map([[1, forgeRecord(1, oid)]]),
    mutationRequests: 0,
    unknownMutationRequests: 0,
    listRequests: 0,
  };
  const server = createServer(async (request, response) => {
    try {
      const url = new URL(request.url, "http://127.0.0.1");
      const match = url.pathname.match(
        /^\/api\/v1\/repos\/owner\/repo\/pulls(?:\/(\d+))?(\/merge)?$/,
      );
      if (!match) return responseJson(response, 404, { message: "not found" });
      const number = match[1] ? Number(match[1]) : null;
      if (request.method === "GET" && number === null) {
        state.listRequests += 1;
        return responseJson(response, 200, [...state.changes.values()]);
      }
      if (request.method === "GET" && number !== null) {
        const record = state.changes.get(number);
        return responseJson(response, record ? 200 : 404, record ?? { message: "missing" });
      }
      const body = await requestJson(request);
      state.mutationRequests += 1;
      if (request.method === "POST" && number === null) {
        const createdNumber = Math.max(...state.changes.keys()) + 1;
        const created = forgeRecord(createdNumber, oid, {
          title: body.title,
          body: body.body,
          head: { ref: body.head, sha: oid },
          base: { ref: body.base },
          updated_at: `revision-${createdNumber}-created`,
        });
        state.changes.set(createdNumber, created);
        if (body.title === "Unknown response create") {
          state.unknownMutationRequests += 1;
          return responseJson(response, 503, { message: "accepted but response lost" });
        }
        return responseJson(response, 201, created);
      }
      const current = state.changes.get(number);
      if (!current) return responseJson(response, 404, { message: "missing" });
      if (request.method === "POST" && match[2] === "/merge") {
        const merged = {
          ...current,
          state: "closed",
          merged: true,
          updated_at: `revision-${number}-merged`,
        };
        state.changes.set(number, merged);
        return responseJson(response, 200, merged);
      }
      if (request.method === "PATCH") {
        const updated = {
          ...current,
          title: body.title ?? current.title,
          body: body.body ?? current.body,
          base: body.base ? { ref: body.base } : current.base,
          state: body.state ?? current.state,
          updated_at: `revision-${number}-${state.mutationRequests}`,
        };
        state.changes.set(number, updated);
        return responseJson(response, 200, updated);
      }
      return responseJson(response, 405, { message: "unsupported" });
    } catch (error) {
      return responseJson(response, 500, { message: String(error) });
    }
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Forge fixture did not bind TCP");
  return { server, state, port: address.port };
}

async function updateForgeCredential(secretRef, secret) {
  await switchToWorkbench();
  const result = await browser.executeAsync(
    (reference, value, done) => {
      window.__TAURI_INTERNALS__
        .invoke("update_forge_credential", {
          request: { secretRef: reference, secret: value },
        })
        .then(
          () => done({ ok: true }),
          (error) => done({ ok: false, error: String(error) }),
        );
    },
    secretRef,
    secret,
  );
  if (!result.ok) throw new Error(`Forge credential update failed: ${result.error}`);
}

async function approveNextSideEffect(timeout = 60_000) {
  await waitForDisplayed('[data-testid="workbench-approve-once"]', timeout);
  await clickWhenReady('[data-testid="workbench-approve-once"]');
}

async function ensureProjectVisible() {
  await switchToWorkbench();
  if (!(await isDisplayed(".project-row"))) {
    await clickWhenReady('[data-testid="workbench-add-project"]');
    await waitForDisplayed(".project-row", 20_000);
  }
  const expanded = await browser.execute(
    () => document.querySelector(".project-row")?.getAttribute("aria-expanded") === "true",
  );
  if (!expanded) await clickWhenReady(".project-row");
}

async function startProjectTask(prompt) {
  await ensureProjectVisible();
  await clickWhenReady('[data-testid^="project-new-task-"]');
  await waitForDisplayed('[data-testid="workbench-composer-input"]');
  await $('[data-testid="workbench-composer-input"]').setValue(prompt);
  await clickWhenReady('[data-testid="workbench-start-task"]');
}

async function waitForRun(status, timeout = 60_000) {
  await browser.waitUntil(
    async () =>
      (await $('[data-testid="workbench-session-timeline"]').getAttribute("data-run-status")) ===
      status,
    {
      timeout,
      timeoutMsg: `Agent Run did not reach ${status}`,
    },
  );
}

async function timelineText() {
  await switchToWorkbench();
  return browser.execute(() => document.querySelector(".timeline-items")?.textContent ?? "");
}

async function waitForTimeline(expected, timeout = 60_000) {
  await browser.waitUntil(async () => (await timelineText()).includes(expected), {
    timeout,
    timeoutMsg: `timeline did not include: ${expected}`,
  });
}

describe("Hachimi Agent product tool reachability", () => {
  let fixtureOid;
  let forgeFixture;
  let forgeRemoteUrl;
  let forgeSecretRef;

  before(async () => {
    ensureGitRemoteFixture();
    const project = process.env.HACHIMI_DESKTOP_E2E_PROJECT_PATH;
    fixtureOid = git(project, "rev-parse", "HEAD");
    const bareRemote = join(dirname(project), "agent-remote.git");
    const bare = spawnSync("git", ["rev-parse", "--is-bare-repository"], {
      cwd: bareRemote,
      encoding: "utf8",
    });
    if (bare.status !== 0) git(project, "init", "--bare", bareRemote);
    setRemote(project, "local-e2e", bareRemote);

    forgeFixture = await startForgeFixture(fixtureOid);
    forgeRemoteUrl = `http://gitea@127.0.0.1:${forgeFixture.port}/owner/repo.git`;
    setRemote(project, "forge-e2e", forgeRemoteUrl);
    const remoteHash = createHash("sha256").update(forgeRemoteUrl).digest("hex");
    forgeSecretRef = `forge:gitea_forgejo:${remoteHash.slice(0, 24)}`;
    await updateForgeCredential(forgeSecretRef, "desktop-e2e-forge-token");
  });

  after(async () => {
    if (forgeSecretRef) await updateForgeCredential(forgeSecretRef, null);
    if (forgeFixture?.server) {
      forgeFixture.server.close();
      await once(forgeFixture.server, "close");
    }
  });

  it("runs spawn, wait, and collect through the real unified ToolPlan", async () => {
    await startProjectTask("[desktop-e2e:multi-agent-tools] run one bounded child task");
    await waitForRun("succeeded");
    await browser.refresh();
    await waitForTimeline("Desktop E2E Coding unified ToolPlan completed");
    const timeline = await $('[data-testid="workbench-session-timeline"]');
    expect(Number(await timeline.getAttribute("data-agent-task-count"))).toBe(1);
    expect(await timeline.getAttribute("data-agent-task-statuses")).toContain("succeeded");
    expect(await timelineText()).toContain("Desktop E2E Coding unified ToolPlan completed");
  });

  it("pushes a local Remote successfully, then fences Remote drift", async () => {
    await startProjectTask(
      `[desktop-e2e:agent-git-forge] oid=${fixtureOid} push the local fixture and verify drift`,
    );
    await approveNextSideEffect();
    await waitForTimeline("forge_remote_drift", 90_000);
    await waitForRun("succeeded", 90_000);
    await browser.refresh();
    await waitForTimeline("successful push and drift fencing completed", 30_000);
    const text = await timelineText();
    expect(text).toContain("git.remotes");
    expect(text).toContain("git.push");
    expect(text).toContain("git_push_confirmed");
    expect(text).toContain("forge_remote_drift");
  });

  it("queries and performs create, update, close, and merge through the Agent Forge tool", async () => {
    const before = forgeFixture.state.mutationRequests;
    await startProjectTask(
      `[desktop-e2e:agent-forge-lifecycle] oid=${fixtureOid} exercise the loopback Forge lifecycle`,
    );
    await approveNextSideEffect(90_000);
    await waitForTimeline("Forge query/create/update/close/merge completed", 120_000);
    await waitForRun("succeeded", 120_000);
    expect(await timelineText()).toContain("forge.change.mutate");
    expect(forgeFixture.state.mutationRequests - before).toBe(4);
  });

  it("reuses one unified and domain receipt for a duplicate Agent mutation", async () => {
    const before = forgeFixture.state.mutationRequests;
    await startProjectTask(
      `[desktop-e2e:agent-forge-duplicate] oid=${fixtureOid} repeat one identical ToolCall`,
    );
    await approveNextSideEffect(90_000);
    await waitForTimeline("duplicate Agent invocation reused one unified side effect", 120_000);
    await waitForRun("succeeded", 120_000);
    expect(forgeFixture.state.mutationRequests - before).toBe(1);
  });

  it("reconciles one unknown Forge mutation without replaying it", async () => {
    const before = forgeFixture.state.unknownMutationRequests;
    await startProjectTask(
      `[desktop-e2e:agent-forge-unknown] oid=${fixtureOid} reconcile one unknown create`,
    );
    await approveNextSideEffect(90_000);
    await waitForTimeline("unknown Forge mutation reconciled without replay", 120_000);
    await waitForRun("succeeded", 120_000);
    expect(forgeFixture.state.unknownMutationRequests - before).toBe(1);
    expect(forgeFixture.state.listRequests).toBeGreaterThan(0);
  });

  it("rejects stale Forge revisions after approval and revoked credentials", async () => {
    await startProjectTask(
      `[desktop-e2e:agent-forge-revision] oid=${fixtureOid} verify concurrent revision fencing`,
    );
    await waitForDisplayed('[data-testid="workbench-approve-once"]', 90_000);
    const current = forgeFixture.state.changes.get(1);
    forgeFixture.state.changes.set(1, { ...current, updated_at: "concurrent-revision" });
    await clickWhenReady('[data-testid="workbench-approve-once"]');
    await waitForTimeline("forge_revision_conflict", 120_000);
    await waitForRun("succeeded", 120_000);

    await updateForgeCredential(forgeSecretRef, null);
    await startProjectTask("[desktop-e2e:agent-forge-credential] verify revoked credentials");
    await waitForTimeline("forge_credential_failed", 90_000);
    await waitForRun("succeeded", 90_000);
    await updateForgeCredential(forgeSecretRef, "desktop-e2e-forge-token");
  });

  it("exposes the Codex-style branch and commit controls while a Project Run is active", async () => {
    await startProjectTask("[desktop-e2e:schedule-wait] keep the Project Run active for Git UI");
    await waitForRun("running", 30_000);
    const summaryPin = await $('[data-testid="workbench-pin-summary"]');
    const pinClasses = (await summaryPin.getAttribute("class"))?.split(/\s+/) ?? [];
    if (!pinClasses.includes("active"))
      await clickWhenReady('[data-testid="workbench-pin-summary"]');
    await waitForDisplayed('[data-testid="workbench-git-branch-trigger"]', 20_000);
    await clickWhenReady('[data-testid="workbench-git-branch-trigger"]');
    await waitForDisplayed(".workbench-git-popover.branch", 20_000);
    await browser.waitUntil(
      async () => /main|master/.test(await $(".workbench-git-popover.branch").getText()),
      { timeout: 20_000, timeoutMsg: "Branch menu did not load the current local branch" },
    );
    await browser.keys(["Escape"]);
    await $(".workbench-git-popover.branch").waitForDisplayed({ reverse: true, timeout: 10_000 });
    await clickWhenReady('[data-testid="workbench-git-commit-trigger"]');
    await waitForDisplayed(".workbench-git-popover.commit", 20_000);
    expect(await $(".workbench-git-popover.commit").getText()).toContain("提交并推送");
    await clickWhenReady('[data-testid="workbench-start-task"]');
    await waitForRun("cancelled", 60_000);
  });

  it("exposes profile-independent enterprise attachment and Multi-Agent capabilities", async () => {
    await switchToWorkbench();
    await clickWhenReady('[data-testid="workbench-new-task"]');
    await $('[data-testid="workbench-composer-input"]').setValue(
      "[desktop-e2e:enterprise-attachment-tool] verify the General ToolPlan matrix",
    );
    await clickWhenReady('[data-testid="workbench-start-task"]');
    await waitForTimeline("General unified ToolPlan exposed", 60_000);
    await waitForRun("succeeded");
    expect(await timelineText()).toContain(
      "General unified ToolPlan exposed profile-independent Multi-Agent and enterprise attachment capabilities.",
    );
  });

  it("runs Multi-Agent through General and Office unified ToolPlans", async () => {
    await switchToWorkbench();
    await clickWhenReady('[data-testid="workbench-new-task"]');
    await $('[data-testid="workbench-composer-input"]').setValue(
      "[desktop-e2e:multi-agent-general] run one bounded General child",
    );
    await clickWhenReady('[data-testid="workbench-start-task"]');
    await waitForTimeline("General unified ToolPlan completed", 90_000);
    await waitForRun("succeeded", 90_000);

    await switchToWorkbench();
    await clickWhenReady('[data-testid="workbench-new-task"]');
    await $('[data-testid="workbench-composer-input"]').setValue(
      "$office-documents [desktop-e2e:multi-agent-office] run one bounded Office child",
    );
    await clickWhenReady('[data-testid="workbench-start-task"]');
    await waitForTimeline("Office unified ToolPlan completed", 90_000);
    await waitForRun("succeeded", 90_000);
  });
});
