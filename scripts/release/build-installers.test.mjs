import assert from "node:assert/strict";
import test from "node:test";

import { deriveMsiVersion } from "./build-installers.mjs";

test("MSI uses a numeric prerelease overlay for the alpha candidate", () => {
  assert.equal(deriveMsiVersion("0.3.0-alpha.8"), "0.3.0-8");
});

test("MSI keeps a stable release version and rejects ambiguous prereleases", () => {
  assert.equal(deriveMsiVersion("0.3.0"), "0.3.0");
  assert.throws(() => deriveMsiVersion("0.3.0-rc.1"), /release_msi_version_unsupported/);
  assert.throws(() => deriveMsiVersion("0.3.0-alpha.65536"), /release_msi_prerelease_out_of_range/);
});
