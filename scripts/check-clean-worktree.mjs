import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function runGit(arguments_) {
  const result = spawnSync("git", arguments_, {
    cwd: repoRoot,
    encoding: "utf8",
    shell: false,
    windowsHide: true,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || "unknown git error").trim();
    throw new Error(`git ${arguments_.join(" ")} failed: ${detail}`);
  }
  return result.stdout.trim();
}

const head = runGit(["rev-parse", "--verify", "HEAD"]);
const status = runGit([
  "status",
  "--porcelain=v1",
  "--untracked-files=all",
  "--ignore-submodules=none",
]);

if (status) {
  const entries = status.split(/\r?\n/u);
  const preview = entries.slice(0, 20).join("\n");
  const remainder = entries.length > 20 ? `\n... and ${entries.length - 20} more` : "";
  throw new Error(
    `Release Gate requires a clean, committed worktree. Commit or remove these changes before retrying:\n${preview}${remainder}`,
  );
}

console.log(`Release worktree is clean at ${head}.`);
