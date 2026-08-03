import { describe, expect, it } from "vitest";

import { resolveLocalMarkdownPath } from "./local-file-links";

describe("resolveLocalMarkdownPath", () => {
  it("routes relative and workspace-contained absolute paths", () => {
    expect(resolveLocalMarkdownPath("./src/main.ts:12", "D:\\repo")).toBe("src/main.ts");
    expect(resolveLocalMarkdownPath("D:/repo/src/main.ts#L9", "D:\\repo")).toBe("src/main.ts");
  });

  it("rejects web links and paths outside the checkout", () => {
    expect(resolveLocalMarkdownPath("https://example.com/a.ts", "D:\\repo")).toBeUndefined();
    expect(resolveLocalMarkdownPath("D:/other/a.ts", "D:\\repo")).toBeUndefined();
    expect(resolveLocalMarkdownPath("../secret.txt", "D:\\repo")).toBeUndefined();
  });
});
