import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const outputRoot = resolve(root, "target/desktop-stress");
const targetRoot = resolve(root, "target");
if (!outputRoot.startsWith(`${targetRoot}${sep}`)) throw new Error("stress output escaped target");

const requested =
  process.argv.find((value) => value.startsWith("--seconds="))?.split("=")[1] ??
  process.env.HACHIMI_DESKTOP_STRESS_SECONDS ??
  "600";
const durationSeconds = Number.parseInt(requested, 10);
if (!Number.isInteger(durationSeconds) || durationSeconds < 1 || durationSeconds > 900) {
  throw new Error("Desktop stress duration must be an integer from 1 through 900 seconds");
}

const mode = process.env.HACHIMI_DESKTOP_STRESS_MODE ?? "real";
if (!new Set(["real", "deterministic"]).has(mode)) {
  throw new Error("HACHIMI_DESKTOP_STRESS_MODE must be real or deterministic");
}
const browserSeconds = Math.max(1, Math.floor(durationSeconds / 2));
const computerSeconds = Math.max(1, durationSeconds - Math.floor(durationSeconds / 2));
const rustStress = (name, crate, test, seconds) => ({
  name,
  seconds,
  command: process.execPath,
  args: [
    "scripts/run-with-rust.mjs",
    "cargo",
    "test",
    "-p",
    crate,
    test,
    "--",
    "--ignored",
    "--nocapture",
    "--test-threads=1",
  ],
});
const phases =
  mode === "real"
    ? [
        {
          name: "browser",
          seconds: browserSeconds,
          command: process.execPath,
          args: ["scripts/desktop-stress/browser-host-real.mjs"],
        },
        rustStress(
          "computer",
          "hachimi-computer",
          "captures_and_controls_the_win32_stress_fixture_with_wgc",
          computerSeconds,
        ),
      ]
    : [
        rustStress(
          "browser",
          "hachimi-browser",
          "short_stress_releases_observations_permissions_and_takeover_state",
          browserSeconds,
        ),
        rustStress(
          "computer",
          "hachimi-computer",
          "short_stress_releases_frames_and_fences_stale_epochs",
          computerSeconds,
        ),
      ];

const processSampleScript = String.raw`
$ErrorActionPreference = "Stop"
$rootPid = [int]$env:HACHIMI_STRESS_ROOT_PID
$all = @(Get-CimInstance Win32_Process -ErrorAction Stop)
$ids = [Collections.Generic.HashSet[int]]::new()
[void]$ids.Add($rootPid)
$changed = $true
while ($changed) {
  $changed = $false
  foreach ($row in $all) {
    if ($ids.Contains([int]$row.ParentProcessId) -and $ids.Add([int]$row.ProcessId)) {
      $changed = $true
    }
  }
}
$rows = @($ids | ForEach-Object {
  $process = Get-Process -Id $_ -ErrorAction SilentlyContinue
  if ($process) {
    [pscustomobject]@{
      pid = $process.Id
      name = $process.ProcessName
      workingSetBytes = [int64]$process.WorkingSet64
      handles = [int64]$process.HandleCount
    }
  }
})
[pscustomobject]@{
  workingSetBytes = [int64](($rows | Measure-Object workingSetBytes -Sum).Sum)
  handles = [int64](($rows | Measure-Object handles -Sum).Sum)
  processes = $rows
} | ConvertTo-Json -Depth 4 -Compress
`;

function sampleProcessTree(pid) {
  if (process.platform !== "win32") {
    return { workingSetBytes: 0, handles: 0, processes: [], unsupported: true };
  }
  const result = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", processSampleScript],
    {
      cwd: root,
      env: { ...process.env, HACHIMI_STRESS_ROOT_PID: String(pid) },
      encoding: "utf8",
      windowsHide: true,
    },
  );
  if (result.status !== 0) throw new Error(`resource sampling failed: ${result.stderr}`);
  return JSON.parse(result.stdout);
}

function terminateTree(pid) {
  if (process.platform === "win32") {
    spawnSync("taskkill.exe", ["/PID", String(pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
  } else {
    try {
      process.kill(pid, "SIGTERM");
    } catch {
      // The process already exited.
    }
  }
}

async function runPhase(phase, phaseReport) {
  const child = spawn(phase.command, phase.args, {
    cwd: root,
    env: { ...process.env, HACHIMI_STRESS_PHASE_SECONDS: String(phase.seconds) },
    stdio: "inherit",
    windowsHide: true,
  });
  const exit = new Promise((resolveExit, rejectExit) => {
    child.once("error", rejectExit);
    child.once("exit", (code) => resolveExit(code));
  });
  const samples = phaseReport.samples;
  const warmupMs = Math.min(30_000, Math.max(2_000, phase.seconds * 100));
  let code;
  try {
    while (code === undefined) {
      const outcome = await Promise.race([
        exit.then((exitCode) => ({ exitCode })),
        new Promise((resolveDelay) => {
          const timer = setTimeout(
            () => resolveDelay({ sample: true }),
            samples.length === 0 ? warmupMs : 60_000,
          );
          timer.unref();
        }),
      ]);
      if ("exitCode" in outcome) {
        code = outcome.exitCode;
        break;
      }
      const sample = { atMs: Date.now(), ...sampleProcessTree(child.pid) };
      samples.push(sample);
      if (sample.workingSetBytes > 2.5 * 1024 ** 3) {
        terminateTree(child.pid);
        throw new Error(`${phase.name}: resource_budget_exceeded`);
      }
    }
  } finally {
    if (child.exitCode == null) terminateTree(child.pid);
  }
  if (code !== 0) throw new Error(`${phase.name} stress failed with exit code ${code}`);
  if (samples.length > 1 && !samples[0].unsupported) {
    const baseline = samples[0];
    const final = samples.at(-1);
    const allowedGrowth = Math.max(baseline.workingSetBytes * 0.25, 256 * 1024 ** 2);
    if (final.workingSetBytes - baseline.workingSetBytes > allowedGrowth) {
      throw new Error(`${phase.name}: working_set_growth_exceeded`);
    }
    if (final.handles - baseline.handles > 100) {
      throw new Error(`${phase.name}: handle_growth_exceeded`);
    }
  }
  return phaseReport;
}

rmSync(outputRoot, { recursive: true, force: true });
mkdirSync(outputRoot, { recursive: true });
if (mode === "real" && process.platform === "win32") {
  const fixtureBuild = spawnSync(
    process.execPath,
    [
      "scripts/run-with-rust.mjs",
      "cargo",
      "build",
      "-p",
      "hachimi-computer",
      "--example",
      "stress_fixture",
    ],
    { cwd: root, stdio: "inherit", windowsHide: true },
  );
  if (fixtureBuild.status !== 0) {
    throw new Error("failed to build the Win32 Computer stress fixture");
  }
  process.env.HACHIMI_COMPUTER_STRESS_FIXTURE = resolve(
    root,
    "target/debug/examples/stress_fixture.exe",
  );
}
const report = {
  durationSeconds,
  mode,
  maxInstances: 1,
  startedAt: new Date().toISOString(),
  phases: [],
};
try {
  for (const phase of phases) {
    const phaseReport = {
      name: phase.name,
      seconds: phase.seconds,
      status: "running",
      samples: [],
    };
    report.phases.push(phaseReport);
    await runPhase(phase, phaseReport);
    phaseReport.status = "passed";
  }
  report.status = "passed";
} catch (error) {
  const failedPhase = report.phases.at(-1);
  if (failedPhase?.status === "running") failedPhase.status = "failed";
  report.status = "failed";
  report.error = error instanceof Error ? error.message : String(error);
  throw error;
} finally {
  report.completedAt = new Date().toISOString();
  writeFileSync(resolve(outputRoot, "report.json"), JSON.stringify(report, null, 2), "utf8");
}
