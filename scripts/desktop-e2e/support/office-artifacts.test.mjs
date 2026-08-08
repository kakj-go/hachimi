import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  createOfficeArtifact,
  createOfficePackageFixtureForTest,
  validateOfficeArtifact,
  validateOfficeOutputTarget,
} from "./office-artifacts.mjs";

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
      if (kind === "docx") {
        assert.equal(created.semantics.title, "Deterministic title");
        assert.deepEqual(created.semantics.paragraphs, ["Verified body"]);
      } else if (kind === "xlsx") {
        assert.deepEqual(created.semantics.sheets, [
          {
            name: "Summary",
            cells: { A1: "Deterministic title", A2: "Verified body" },
          },
        ]);
      } else if (kind === "pptx") {
        assert.deepEqual(created.semantics.slides[0].texts, [
          "Deterministic title",
          "Verified body",
        ]);
      } else {
        assert.deepEqual(created.semantics.pages[0].texts, [
          "Deterministic title",
          "Verified body",
        ]);
      }
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("validation rejects traversal, duplicate parts, macros, damaged ZIPs and zip bombs", () => {
  const root = mkdtempSync(join(tmpdir(), "hachimi-office-security-"));
  const contentTypes =
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>';
  const required = [
    ["[Content_Types].xml", contentTypes],
    ["_rels/.rels", "<Relationships/>"],
    ["word/document.xml", '<w:document xmlns:w="w"><w:t>safe</w:t></w:document>'],
  ];
  try {
    const writePackage = (name, entries) => {
      const path = join(root, name);
      writeFileSync(path, createOfficePackageFixtureForTest(entries));
      return path;
    };
    assert.throws(
      () => validateOfficeArtifact(writePackage("traversal.docx", [["../escape.xml", "x"]])),
      /path traversal/u,
    );
    assert.throws(
      () =>
        validateOfficeArtifact(
          writePackage("duplicate.docx", [
            ["same.xml", "one"],
            ["same.xml", "two"],
          ]),
        ),
      /duplicate part/u,
    );
    assert.throws(
      () =>
        validateOfficeArtifact(
          writePackage("macro.docx", [...required, ["word/vbaProject.bin", Buffer.from([1])]]),
        ),
      /macro content/u,
    );
    const damaged = createOfficePackageFixtureForTest(required).subarray(0, 35);
    writeFileSync(join(root, "damaged.docx"), damaged);
    assert.throws(() => validateOfficeArtifact(join(root, "damaged.docx")), /truncated/u);

    const bomb = createOfficePackageFixtureForTest([["bomb.xml", "x"]]);
    bomb.writeUInt16LE(8, 8);
    bomb.writeUInt32LE(1_000_000, 22);
    writeFileSync(join(root, "bomb.docx"), bomb);
    assert.throws(() => validateOfficeArtifact(join(root, "bomb.docx")), /compression ratio/u);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("Office output targets reject traversal and overwrite conflicts", () => {
  const root = mkdtempSync(join(tmpdir(), "hachimi-office-output-"));
  try {
    const existing = join(root, "existing.docx");
    writeFileSync(existing, "fixture");
    assert.throws(
      () => validateOfficeOutputTarget(root, join(root, "..", "outside.docx")),
      /escaped/u,
    );
    assert.throws(() => validateOfficeOutputTarget(root, existing), /overwrite/u);
    assert.equal(validateOfficeOutputTarget(root, existing, { overwrite: true }), existing);
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
