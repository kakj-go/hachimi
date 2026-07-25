import { createHash } from "node:crypto";
import { createReadStream, existsSync, readFileSync, statSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const scriptsDirectory = dirname(fileURLToPath(import.meta.url));
const workspaceRoot = resolve(scriptsDirectory, "..");
const modelRoot = join(workspaceRoot, "apps", "desktop", "src-tauri", "resources", "ai-models");
const manifests = [
  join(modelRoot, "speech-to-text", "sensevoice-small", "manifest.json"),
  join(modelRoot, "text-to-speech", "vits-melo-zh-en", "manifest.json"),
];

async function sha256(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

function containedFile(manifestPath, name) {
  if (isAbsolute(name)) throw new Error(`Absolute model path is forbidden: ${name}`);
  const root = dirname(manifestPath);
  const path = resolve(root, name);
  const rel = relative(root, path);
  if (!rel || rel === ".." || rel.startsWith(`..${sep}`) || isAbsolute(rel)) {
    throw new Error(`Model path escapes its manifest directory: ${name}`);
  }
  return path;
}

async function verifyModels() {
  const failures = [];
  for (const manifestPath of manifests) {
    if (!existsSync(manifestPath)) {
      failures.push(`missing manifest ${relative(workspaceRoot, manifestPath)}`);
      continue;
    }
    let manifest;
    try {
      manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
    } catch (error) {
      failures.push(`invalid manifest ${relative(workspaceRoot, manifestPath)}: ${error}`);
      continue;
    }
    if (!manifest.files || typeof manifest.files !== "object") {
      failures.push(`manifest has no verified files: ${relative(workspaceRoot, manifestPath)}`);
      continue;
    }
    for (const [name, expected] of Object.entries(manifest.files)) {
      try {
        const path = containedFile(manifestPath, name);
        if (!existsSync(path)) {
          failures.push(`missing ${relative(workspaceRoot, path)}`);
          continue;
        }
        const stats = statSync(path);
        if (!stats.isFile() || stats.size !== expected.size) {
          failures.push(`invalid size for ${relative(workspaceRoot, path)}`);
          continue;
        }
        const actual = await sha256(path);
        if (actual !== expected.sha256) {
          failures.push(`SHA-256 mismatch for ${relative(workspaceRoot, path)}`);
        }
      } catch (error) {
        failures.push(String(error));
      }
    }
  }
  return failures;
}

let failures = await verifyModels();
if (failures.length === 0) {
  console.log("Bundled speech models verified.");
  process.exit(0);
}

console.warn(`Speech models need preparation:\n- ${failures.join("\n- ")}`);
const shell = process.platform === "win32" ? "powershell.exe" : "pwsh";
const preparation = spawnSync(
  shell,
  [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    join(scriptsDirectory, "prepare-speech-models.ps1"),
  ],
  { cwd: workspaceRoot, stdio: "inherit", windowsHide: true },
);
if (preparation.error) {
  throw new Error(`Unable to start ${shell}: ${preparation.error.message}`);
}
if (preparation.status !== 0) {
  throw new Error(`Speech model preparation failed with exit code ${preparation.status}.`);
}

failures = await verifyModels();
if (failures.length !== 0) {
  throw new Error(`Prepared speech models failed verification:\n- ${failures.join("\n- ")}`);
}
console.log("Bundled speech models prepared and verified.");
