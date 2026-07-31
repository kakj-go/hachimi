import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const workspaceRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const tauriConfig = "apps/desktop/src-tauri/tauri.conf.json";

export function deriveMsiVersion(sourceVersion) {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-alpha\.(\d+))?$/.exec(sourceVersion);
  if (!match) throw new Error(`release_msi_version_unsupported:${sourceVersion}`);
  if (match[4] === undefined) return sourceVersion;
  const prerelease = Number(match[4]);
  if (!Number.isSafeInteger(prerelease) || prerelease > 65_535) {
    throw new Error(`release_msi_prerelease_out_of_range:${match[4]}`);
  }
  return `${match[1]}.${match[2]}.${match[3]}-${prerelease}`;
}

function runTauri(arguments_) {
  const result = spawnSync(
    process.execPath,
    ["scripts/run-with-rust.mjs", "tauri", ...arguments_],
    {
      cwd: workspaceRoot,
      env: process.env,
      stdio: "inherit",
      windowsHide: true,
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`release_installer_build_failed:${arguments_[1]}:${result.status}`);
  }
}

function buildInstallers() {
  const sourceVersion = JSON.parse(
    readFileSync(resolve(workspaceRoot, "package.json"), "utf8"),
  ).version;
  const msiVersion = deriveMsiVersion(sourceVersion);
  process.stdout.write(`Building NSIS with source version ${sourceVersion}.\n`);
  runTauri(["build", "--bundles", "nsis", "--config", tauriConfig]);
  process.stdout.write(`Building MSI with numeric prerelease overlay ${msiVersion}.\n`);
  runTauri([
    "build",
    "--bundles",
    "msi",
    "--config",
    tauriConfig,
    "--config",
    JSON.stringify({ version: msiVersion }),
  ]);
}

const isCli = process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1]);
if (isCli) buildInstallers();
