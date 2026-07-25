import { describe, expect, it } from "vitest";
import { enUs, zhCn } from "./messages";

describe("message dictionaries", () => {
  it("contain exactly the same keys", () => {
    expect(Object.keys(enUs).sort()).toEqual(Object.keys(zhCn).sort());
  });
});
