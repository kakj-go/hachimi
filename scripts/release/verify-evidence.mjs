import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { verifyArtifactManifest, verifyEvidence, writeEvidence } from "./evidence.mjs";
import { namedCliArguments } from "./cli-args.mjs";

const workspaceRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const args = namedCliArguments(process.argv.slice(2));
const root = resolve(workspaceRoot, args.get("--root") ?? "target/release-evidence");
const output = resolve(root, args.get("--output") ?? "release-manifest.json");
const required = (
  args.get("--required") ??
  "openai,forge,enterprise,channels,windows_standard_user,windows_elevated"
)
  .split(",")
  .filter(Boolean);
let manifest = verifyEvidence(root, {
  required,
  maxAgeHours: Number(args.get("--max-age-hours") ?? 168),
});
const artifactManifestPath = args.get("--artifact-manifest");
if (artifactManifestPath) {
  const artifactManifest = JSON.parse(
    readFileSync(resolve(workspaceRoot, artifactManifestPath), "utf8"),
  );
  manifest = verifyArtifactManifest(artifactManifest, manifest, {
    expectedVersion: args.get("--expected-version"),
    expectedCommit:
      args.get("--expected-commit") ??
      execFileSync("git", ["rev-parse", "HEAD"], {
        cwd: workspaceRoot,
        encoding: "utf8",
        windowsHide: true,
      }).trim(),
  });
}
writeEvidence(output, manifest);
process.stdout.write(`${output}\n`);
