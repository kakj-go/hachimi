import { readdirSync, readFileSync } from "node:fs";
import { extname, relative, resolve } from "node:path";
import process from "node:process";

const workspaceRoot = resolve(import.meta.dirname, "..");
const provenancePath = resolve(workspaceRoot, "docs", "HARNESS_AGENT_SOURCE_PROVENANCE.md");
const provenance = readFileSync(provenancePath, "utf8").replaceAll("\\", "/");
const roots = ["apps", "crates", "packages"].map((entry) => resolve(workspaceRoot, entry));
const sourceExtensions = new Set([".rs", ".sql", ".ts", ".tsx", ".js", ".mjs"]);
const derivedMarker = /\b(?:Adapted from|Translated from|Modified for Hachimi:)\b/u;
const ignoredDirectories = new Set(["binaries", "dist", "gen", "node_modules", "target"]);
const fixedSources = [
  "4c43465133428898aa84f0bfc02c306ed65fb66a",
  "f6d456235cf011004f7cffc71a95acf6fbf1fa0a",
  "34b3dc99bf40c57c0b78f3b5b1d70471ebc2d06d",
];

const sourceFiles = [];
for (const root of roots) walk(root, sourceFiles);

const failures = [];
for (const file of sourceFiles) {
  const contents = readFileSync(file, "utf8");
  if (!derivedMarker.test(contents)) continue;
  const target = relative(workspaceRoot, file).replaceAll("\\", "/");
  if (!provenance.includes(`\`${target}\``)) {
    failures.push(`${target}: contains a third-party adaptation marker but has no provenance row`);
  }
  if (!/^(?:\/\/|--) SPDX-License-Identifier:/mu.test(contents)) {
    failures.push(`${target}: derived source is missing an SPDX header`);
  }
  if (!fixedSources.some((commit) => contents.includes(commit))) {
    failures.push(`${target}: derived source does not identify a fixed source commit`);
  }
}

for (const commit of fixedSources) {
  if (!provenance.includes(commit)) {
    failures.push(`provenance document is missing fixed source commit ${commit}`);
  }
}

if (failures.length > 0) {
  process.stderr.write(`${failures.join("\n")}\n`);
  process.exit(1);
}
process.stdout.write(`Harness provenance check passed for ${sourceFiles.length} source files.\n`);

function walk(directory, output) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory() && ignoredDirectories.has(entry.name)) continue;
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) {
      walk(path, output);
    } else if (entry.isFile() && sourceExtensions.has(extname(entry.name))) {
      output.push(path);
    }
  }
}
