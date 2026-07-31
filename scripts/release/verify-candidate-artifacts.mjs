import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { verifyCandidateArtifacts } from "./evidence.mjs";
import { namedCliArguments } from "./cli-args.mjs";

const workspaceRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const args = namedCliArguments(process.argv.slice(2));
const candidateRoot = resolve(workspaceRoot, args.get("--root") ?? "target/release-candidate");
const manifest = JSON.parse(
  readFileSync(resolve(candidateRoot, args.get("--manifest") ?? "artifact-manifest.json"), "utf8"),
);
const artifactPaths = readdirSync(candidateRoot)
  .filter((name) => /\.(exe|msi|zip)$/i.test(name))
  .map((name) => join(candidateRoot, name));
const expectedVersion =
  args.get("--expected-version") ??
  JSON.parse(readFileSync(resolve(workspaceRoot, "package.json"), "utf8")).version;
const expectedCommit =
  args.get("--expected-commit") ??
  execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: workspaceRoot,
    encoding: "utf8",
    windowsHide: true,
  }).trim();
verifyCandidateArtifacts(manifest, workspaceRoot, artifactPaths, {
  expectedVersion,
  expectedCommit,
});
process.stdout.write(
  `${JSON.stringify({ version: manifest.version, commitSha: manifest.commitSha })}\n`,
);
