import { existsSync, readdirSync, readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { extname, relative, resolve } from "node:path";
import process from "node:process";

const workspaceRoot = resolve(import.meta.dirname, "..");
const provenancePath = resolve(workspaceRoot, "docs", "HARNESS_AGENT_SOURCE_PROVENANCE.md");
const provenance = readFileSync(provenancePath, "utf8").replaceAll("\\", "/");
const registryDefinitions = [
  {
    path: "docs/references/openai/registry.json",
    id: /^OAI-PRODUCT-[A-Z]+-\d{8}$/u,
    url: /^https:\/\/(?:developers\.openai\.com|learn\.chatgpt\.com)\//u,
    requireBehavior: true,
  },
  {
    path: "docs/references/forge/registry.json",
    id: /^(?:GITHUB|GITLAB|GITEE|GITEA-FORGEJO)-API-\d{8}$/u,
    url: /^https:\/\/(?:docs\.github\.com|docs\.gitlab\.com|gitee\.com|docs\.gitea\.com|forgejo\.org|codeberg\.org)\//u,
    requireBehavior: false,
  },
  {
    path: "docs/references/enterprise/registry.json",
    id: /^(?:(?:WECOM|DINGTALK|FEISHU)-API|DINGTALK-STREAM-SDK-GO|FEISHU-SDK-GO)-\d{8}$/u,
    url: /^https:\/\/(?:developer\.work\.weixin\.qq\.com|open\.dingtalk\.com|open\.feishu\.cn|github\.com\/(?:open-dingtalk\/dingtalk-stream-sdk-go|larksuite\/oapi-sdk-go))\//u,
    requireBehavior: false,
  },
];
const roots = ["apps", "crates", "packages"].map((entry) => resolve(workspaceRoot, entry));
const sourceExtensions = new Set([".rs", ".sql", ".ts", ".tsx", ".js", ".mjs"]);
const derivedMarker = /\b(?:Adapted from|Translated from|Modified for Hachimi:)\b/u;
const ignoredDirectories = new Set(["binaries", "dist", "gen", "node_modules", "target"]);
const fixedSources = [
  "4c43465133428898aa84f0bfc02c306ed65fb66a",
  "f6d456235cf011004f7cffc71a95acf6fbf1fa0a",
  "34b3dc99bf40c57c0b78f3b5b1d70471ebc2d06d",
  "11bb8ddcd95f41e673783b7e20c4ab4cd5ee7e24",
  "f2e4ed83c6953297a92d66d49114196fc4402206",
  "7820949d3a3d8ca0b6f07d060deab40cb86807ff",
  "03a6edb29e7e742fb97d5926949c03c3214a31e5",
  "848a1a4eb171f3d891304370f7d5ed42046f3280",
  "d1cc841e6013c3f6513a5bb01dfe3219b9c37d17",
  "ff207b774541a195f0a98c5bfda1507905e45431",
];

const sourceFiles = [];
for (const root of roots) walk(root, sourceFiles);

const failures = [];
const referenceIds = new Set();
for (const definition of registryDefinitions) {
  const registryPath = resolve(workspaceRoot, definition.path);
  if (!existsSync(registryPath)) {
    failures.push(`missing source registry: ${definition.path}`);
    continue;
  }
  const registry = JSON.parse(readFileSync(registryPath, "utf8"));
  for (const reference of registry.references ?? []) {
    if (!definition.id.test(reference.id) || referenceIds.has(reference.id)) {
      failures.push(`invalid or duplicate product reference ID: ${reference.id}`);
      continue;
    }
    referenceIds.add(reference.id);
    if (!definition.url.test(reference.canonicalUrl)) {
      failures.push(`${reference.id}: canonicalUrl is not an allowed official documentation URL`);
    }
    if (Number.isNaN(Date.parse(reference.retrievedAt))) {
      failures.push(`${reference.id}: retrievedAt is not an ISO timestamp`);
    }
    for (const [pathField, hashField] of [
      ["rawPath", "rawSha256"],
      ["normalizedPath", "normalizedSha256"],
    ]) {
      const relativePath = reference[pathField];
      const expectedHash = reference[hashField];
      const absolutePath = resolve(workspaceRoot, relativePath ?? "");
      if (!relativePath || !existsSync(absolutePath)) {
        failures.push(
          `${reference.id}: missing ${pathField} snapshot ${relativePath ?? "<unset>"}`,
        );
        continue;
      }
      const actualHash = createHash("sha256").update(readFileSync(absolutePath)).digest("hex");
      if (actualHash !== expectedHash) {
        failures.push(
          `${reference.id}: ${hashField} mismatch (expected ${expectedHash}, got ${actualHash})`,
        );
      }
    }
    if (definition.requireBehavior && (!reference.hachimiBehavior || !reference.acceptance)) {
      failures.push(`${reference.id}: missing Hachimi behavior or acceptance mapping`);
    }
  }
}

const channelRegistryPath = resolve(
  workspaceRoot,
  "docs",
  "references",
  "channels",
  "registry.json",
);
if (!existsSync(channelRegistryPath)) {
  failures.push("missing source registry: docs/references/channels/registry.json");
} else {
  const registry = JSON.parse(readFileSync(channelRegistryPath, "utf8"));
  if (registry.schemaVersion !== 1 || !Array.isArray(registry.sources)) {
    failures.push("invalid Channel source registry shape");
  } else {
    const names = new Set();
    for (const source of registry.sources) {
      if (
        typeof source.name !== "string" ||
        names.has(source.name) ||
        !/^https:\/\/github\.com\//u.test(source.repository ?? "") ||
        !/^[a-f0-9]{40}$/iu.test(source.commit ?? "") ||
        typeof source.license !== "string" ||
        !["copied", "adapted", "behavior-only"].includes(source.classification) ||
        !Array.isArray(source.files) ||
        source.files.length === 0 ||
        typeof source.implementationStatus !== "string"
      ) {
        failures.push(`invalid Channel source entry: ${source.name ?? "missing"}`);
        continue;
      }
      names.add(source.name);
      if (!fixedSources.includes(source.commit)) {
        failures.push(`unregistered Channel source commit: ${source.name}`);
      }
    }
  }
}

const behaviorDocuments = [
  "docs/ROADMAP.md",
  "docs/HACHIMI_AI_IMPLEMENTATION_SPEC.md",
  "docs/HARNESS_AGENT_ARCHITECTURE_AND_IMPLEMENTATION.md",
  "docs/HARNESS_AGENT_CODE_IMPLEMENTATION_PLAN.md",
  "docs/HARNESS_AGENT_SOURCE_PROVENANCE.md",
];
for (const documentPath of behaviorDocuments) {
  const contents = readFileSync(resolve(workspaceRoot, documentPath), "utf8");
  const cited = new Set([...contents.matchAll(/\[ref:([A-Z0-9-]+)\]/gu)].map((match) => match[1]));
  for (const id of cited) {
    if (!referenceIds.has(id))
      failures.push(`${documentPath}: unknown product behavior reference ${id}`);
  }
  if (
    /(?:Browser|Chrome|Computer|Plugin|Scheduled Tasks|计划任务|定时任务)/u.test(contents) &&
    cited.size === 0
  ) {
    failures.push(`${documentPath}: Agent product behavior claims must cite a registered [ref:ID]`);
  }
}
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
