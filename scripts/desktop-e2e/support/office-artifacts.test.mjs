import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { createOfficeArtifact, validateOfficeArtifact } from "./office-artifacts.mjs";

test("Desktop Office fixtures are real OOXML and PDF containers", () => {
  const root = mkdtempSync(join(tmpdir(), "hachimi-office-fixture-"));
  try {
    for (const kind of ["docx", "xlsx", "pptx", "pdf"]) {
      const path = join(root, `artifact.${kind}`);
      const created = createOfficeArtifact(path, kind, "Deterministic title", "Verified body");
      assert.equal(created.kind, kind);
      assert.ok(created.byteLength > 100);
      assert.deepEqual(validateOfficeArtifact(path), created);
      assert.notEqual(readFileSync(path, "utf8").trimStart()[0], "{");
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("validation rejects a JSON file with an Office extension", () => {
  const root = mkdtempSync(join(tmpdir(), "hachimi-office-invalid-"));
  try {
    const path = join(root, "fake.docx");
    writeFileSync(path, JSON.stringify({ validated: true }));
    assert.throws(() => validateOfficeArtifact(path), /OOXML ZIP package/u);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
