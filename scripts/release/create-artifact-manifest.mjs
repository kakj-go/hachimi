import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  collectArtifactDigests,
  collectSourceRegistryDigests,
  writeEvidence,
} from "./evidence.mjs";

const workspaceRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const artifactRoot = resolve(workspaceRoot, process.argv[2] ?? "target/release-candidate");
const version = JSON.parse(readFileSync(resolve(workspaceRoot, "package.json"), "utf8")).version;

function artifactFiles(root) {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    if (entry.isDirectory()) return artifactFiles(path);
    return /\.(exe|msi|zip)$/i.test(entry.name) ? [path] : [];
  });
}

if (!statSync(artifactRoot).isDirectory()) throw new Error("release_artifact_root_missing");
const artifacts = artifactFiles(artifactRoot);
if (artifacts.length < 3) throw new Error("release_artifact_set_incomplete");
const manifest = {
  schemaVersion: 1,
  version,
  commitSha: execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: workspaceRoot,
    encoding: "utf8",
    windowsHide: true,
  }).trim(),
  artifactSha256: collectArtifactDigests(artifacts),
  sourceRegistrySha256: collectSourceRegistryDigests(workspaceRoot),
  licenseFiles: ["LICENSE", "NOTICE.md"],
  licenseSha256: collectArtifactDigests([
    resolve(workspaceRoot, "LICENSE"),
    resolve(workspaceRoot, "NOTICE.md"),
  ]),
  unsigned: true,
  generatedAtUtc: new Date().toISOString(),
};
const output = join(artifactRoot, "artifact-manifest.json");
writeEvidence(output, manifest);
process.stdout.write(`${basename(output)}\n`);
