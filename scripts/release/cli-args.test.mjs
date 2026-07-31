import assert from "node:assert/strict";
import test from "node:test";

import { forwardedCliArguments, namedCliArguments } from "./cli-args.mjs";

test("release CLI accepts pnpm's forwarded separator", () => {
  assert.deepEqual(forwardedCliArguments(["--", "target/release-candidate"]), [
    "target/release-candidate",
  ]);
  assert.deepEqual(
    [...namedCliArguments(["--", "--root", "target/release-candidate"])],
    [["--root", "target/release-candidate"]],
  );
});

test("release CLI rejects incomplete or positional named arguments", () => {
  assert.throws(() => namedCliArguments(["--root"]), /release_cli_arguments_invalid/);
  assert.throws(() => namedCliArguments(["root", "value"]), /release_cli_argument_name_invalid/);
});
