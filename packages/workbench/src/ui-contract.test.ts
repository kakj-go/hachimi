import { readdirSync, readFileSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

function filesBelow(root: string): string[] {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    return entry.isDirectory() ? filesBelow(path) : [path];
  });
}

describe("shared UI contract", () => {
  it("keeps native form controls inside @hachimi/ui except hidden file pickers", () => {
    const sourceRoot = import.meta.dirname;
    const violations = filesBelow(sourceRoot)
      .filter((path) => path.endsWith(".tsx") && !path.endsWith(".test.tsx"))
      .flatMap((path) => {
        const source = readFileSync(path, "utf8");
        return [...source.matchAll(/<(button|input|select|textarea)\b/g)]
          .filter((match) => {
            const remainder = source.slice(match.index);
            const closingIndex = remainder.indexOf("/>");
            const tag = closingIndex >= 0 ? remainder.slice(0, closingIndex + 2) : remainder;
            return !(
              match[1] === "input" &&
              /type=["']file["']/.test(tag) &&
              /data-component=["']file-input["']/.test(tag)
            );
          })
          .map((match) => `${path}:${source.slice(0, match.index).split("\n").length}`);
      });
    expect(violations).toEqual([]);
  });

  it("keeps product CSS on shared color, type and radius tokens", () => {
    const sourceRoot = import.meta.dirname;
    const violations = filesBelow(sourceRoot)
      .filter((path) => path.endsWith(".css"))
      .flatMap((path) => {
        const source = readFileSync(path, "utf8");
        const patterns = [
          /#[\dA-Fa-f]{3,8}\b/g,
          /font-size\s*:\s*\d+(?:\.\d+)?px/g,
          /border-radius\s*:\s*(?!0(?:\s|;)|50%|999px)\d+(?:\.\d+)?px/g,
        ];
        return patterns.flatMap((pattern) =>
          [...source.matchAll(pattern)].map(
            (match) => `${path}:${source.slice(0, match.index).split("\n").length}:${match[0]}`,
          ),
        );
      });
    expect(violations).toEqual([]);
  });

  it("keeps every frontend source file below 2000 lines", () => {
    const workspaceRoot = resolve(import.meta.dirname, "../../..");
    const roots = [
      resolve(workspaceRoot, "apps/desktop/web/src"),
      resolve(workspaceRoot, "packages/avatar-motion-runtime/src"),
      resolve(workspaceRoot, "packages/contracts/src"),
      resolve(workspaceRoot, "packages/i18n/src"),
      resolve(workspaceRoot, "packages/pet/src"),
      resolve(workspaceRoot, "packages/settings/src"),
      resolve(workspaceRoot, "packages/ui/src"),
      resolve(workspaceRoot, "packages/workbench/src"),
      resolve(workspaceRoot, "docs/ui-style-demos"),
    ];
    const frontendFiles = [
      ...roots.flatMap(filesBelow),
      resolve(workspaceRoot, "apps/desktop/web/pet.html"),
      resolve(workspaceRoot, "apps/desktop/web/workbench.html"),
      resolve(workspaceRoot, "apps/desktop/web/vite.config.ts"),
    ];
    const oversized = frontendFiles
      .filter((path) => [".css", ".html", ".js", ".ts", ".tsx"].includes(extname(path)))
      .map((path) => ({ path, lines: readFileSync(path, "utf8").split("\n").length }))
      .filter((entry) => entry.lines > 2_000);
    expect(oversized).toEqual([]);
  });

  it("keeps every Rust source file below 2000 lines", () => {
    const workspaceRoot = resolve(import.meta.dirname, "../../..");
    const roots = [
      resolve(workspaceRoot, "apps/desktop/src-tauri/src"),
      resolve(workspaceRoot, "crates"),
    ];
    const oversized = roots
      .flatMap(filesBelow)
      .filter((path) => path.endsWith(".rs"))
      .map((path) => ({ path, lines: readFileSync(path, "utf8").split("\n").length }))
      .filter((entry) => entry.lines > 2_000);
    expect(oversized).toEqual([]);
  });
});
