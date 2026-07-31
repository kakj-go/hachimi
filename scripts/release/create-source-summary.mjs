import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { collectSourceRegistryDigests, writeEvidence } from "./evidence.mjs";
import { forwardedCliArguments } from "./cli-args.mjs";

function required(value, code) {
  if (typeof value !== "string" || !value.trim()) throw new Error(code);
}

function digest(value, code) {
  if (typeof value !== "string" || !/^[a-f0-9]{64}$/i.test(value)) throw new Error(code);
}

export function buildSourceSummary(workspaceRoot) {
  const registryDigests = collectSourceRegistryDigests(workspaceRoot);
  const registries = {};
  for (const area of ["openai", "forge", "enterprise"]) {
    const registry = JSON.parse(
      readFileSync(resolve(workspaceRoot, "docs", "references", area, "registry.json"), "utf8"),
    );
    if (
      registry.schemaVersion !== 1 ||
      !Array.isArray(registry.references) ||
      !registry.references.length
    ) {
      throw new Error(`release_source_registry_invalid:${area}`);
    }
    const ids = new Set();
    registries[area] = {
      sha256: registryDigests[area],
      references: registry.references.map((reference) => {
        for (const key of ["id", "sourceProduct", "sourceVersion", "canonicalUrl", "retrievedAt"]) {
          required(reference[key], `release_source_reference_missing:${area}:${key}`);
        }
        if (ids.has(reference.id)) throw new Error(`release_source_reference_duplicate:${area}`);
        ids.add(reference.id);
        let canonical;
        try {
          canonical = new URL(reference.canonicalUrl);
        } catch {
          throw new Error(`release_source_reference_url_invalid:${area}:${reference.id}`);
        }
        if (canonical.protocol !== "https:") {
          throw new Error(`release_source_reference_url_invalid:${area}:${reference.id}`);
        }
        if (!Number.isFinite(Date.parse(reference.retrievedAt))) {
          throw new Error(`release_source_reference_time_invalid:${area}:${reference.id}`);
        }
        digest(
          reference.rawSha256,
          `release_source_reference_raw_digest_invalid:${area}:${reference.id}`,
        );
        digest(
          reference.normalizedSha256,
          `release_source_reference_normalized_digest_invalid:${area}:${reference.id}`,
        );
        return {
          id: reference.id,
          sourceProduct: reference.sourceProduct,
          sourceVersion: reference.sourceVersion,
          canonicalUrl: reference.canonicalUrl,
          license: reference.license ?? null,
          retrievedAt: reference.retrievedAt,
          rawSha256: reference.rawSha256,
          normalizedSha256: reference.normalizedSha256,
        };
      }),
    };
  }
  return { schemaVersion: 1, registries };
}

const isCli = process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1]);
if (isCli) {
  const workspaceRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));
  const [outputArgument] = forwardedCliArguments(process.argv.slice(2));
  const output = resolve(workspaceRoot, outputArgument ?? "target/release-source-summary.json");
  writeEvidence(output, buildSourceSummary(workspaceRoot));
  process.stdout.write(`${output}\n`);
}
