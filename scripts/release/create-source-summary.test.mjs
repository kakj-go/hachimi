import assert from "node:assert/strict";
import { resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { buildSourceSummary } from "./create-source-summary.mjs";

const workspaceRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));

test("release source summary is bounded to registered public provenance", () => {
  const summary = buildSourceSummary(workspaceRoot);
  assert.equal(summary.schemaVersion, 1);
  assert.deepEqual(Object.keys(summary.registries).sort(), [
    "channels",
    "enterprise",
    "forge",
    "openai",
  ]);
  for (const [area, registry] of Object.entries(summary.registries)) {
    assert.match(registry.sha256, /^[a-f0-9]{64}$/);
    if (area === "channels") {
      assert.ok(registry.sources.length > 0);
      for (const source of registry.sources) {
        assert.match(source.repository, /^https:\/\/github\.com\//);
        assert.match(source.commit, /^[a-f0-9]{40}$/);
        assert.ok(source.files.length > 0);
      }
      continue;
    }
    assert.ok(registry.references.length > 0);
    for (const reference of registry.references) {
      assert.match(reference.canonicalUrl, /^https:\/\//);
      assert.match(reference.rawSha256, /^[a-f0-9]{64}$/);
      assert.match(reference.normalizedSha256, /^[a-f0-9]{64}$/);
      assert.equal("rawPath" in reference, false);
      assert.equal("normalizedPath" in reference, false);
    }
  }
});
