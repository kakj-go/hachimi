import assert from "node:assert/strict";
import { resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { readReleaseVersions, verifyReleaseVersion } from "./version-check.mjs";

const workspaceRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));

test("release version and license metadata are consistent across the workspace", () => {
  const versions = readReleaseVersions(workspaceRoot);
  assert.equal(versions.package, "1.0.0");
  assert.equal(versions.package, versions.cargo);
  assert.equal(versions.package, versions.tauri);
  assert.equal(versions.packageLicense, "Apache-2.0");
  assert.equal(versions.cargoLicense, "Apache-2.0");
  assert.ok(versions.jsPackages.length >= 9);
  assert.ok(
    versions.jsPackages.every(
      (pkg) => pkg.version === versions.package && pkg.license === "Apache-2.0",
    ),
  );

  const result = verifyReleaseVersion(workspaceRoot, "v1.0.0");
  assert.equal(result.version, "1.0.0");
  assert.equal(result.msiVersion, "1.0.0");
  assert.equal(result.license, "Apache-2.0");
});

test("release version check rejects a tag that would overwrite another version", () => {
  assert.throws(
    () => verifyReleaseVersion(workspaceRoot, "v1.0.1"),
    /release_tag_version_mismatch/,
  );
});
