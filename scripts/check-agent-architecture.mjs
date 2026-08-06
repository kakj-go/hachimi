import { readFileSync } from "node:fs";
import { extname, relative, resolve } from "node:path";
import { execFileSync } from "node:child_process";
import process from "node:process";

const root = resolve(import.meta.dirname, "..");
const listed = execFileSync(
  "rg",
  [
    "--files",
    "apps",
    "crates",
    "packages",
    "-g",
    "*.rs",
    "-g",
    "*.ts",
    "-g",
    "*.tsx",
    "-g",
    "*.css",
    "-g",
    "!**/target/**",
    "-g",
    "!**/node_modules/**",
    "-g",
    "!packages/contracts/src/generated.ts",
  ],
  { cwd: root, encoding: "utf8" },
)
  .trim()
  .split(/\r?\n/u)
  .filter(Boolean);

const failures = [];
const globallyForbidden = [
  ["AgentRunExecutor::execute_registered", "legacy closure Agent Runtime"],
  ["InMemoryAudit", "non-persistent production Audit"],
  ["poll_agent_events", "polling Agent command"],
  ["pollAgentEvents", "polling Agent frontend adapter"],
  ["skill_read_resource", "selected-only Skill reader"],
  ["READ_SKILL_RESOURCE_TOOL", "selected-only Skill reader constant"],
];
const entryAssembly = ["ToolLoopDriver::new", "ToolRegistry::new", "SemanticCompactor::new"];

for (const file of listed) {
  const absolute = resolve(root, file);
  const contents = readFileSync(absolute, "utf8");
  const normalized = relative(root, absolute).replaceAll("\\", "/");
  for (const [needle, label] of globallyForbidden) {
    if (contents.includes(needle)) failures.push(`${normalized}: contains ${label} (${needle})`);
  }
  if (
    (normalized.startsWith("apps/desktop/src-tauri/src/") ||
      normalized.startsWith("crates/hachimi-scheduler/src/")) &&
    entryAssembly.some((needle) => contents.includes(needle))
  ) {
    failures.push(`${normalized}: assembles Agent ToolLoop/Registry/Compactor outside the kernel`);
  }
  if (
    normalized === "packages/workbench/src/home.tsx" &&
    /setInterval[\s\S]{0,80}700/u.test(contents)
  ) {
    failures.push(`${normalized}: contains the retired 700ms Agent event polling loop`);
  }
  if (
    normalized.startsWith("packages/workbench/src/") &&
    !normalized.endsWith(".test.ts") &&
    !normalized.endsWith(".test.tsx") &&
    /protocolVersion\s*:\s*\d+/u.test(contents)
  ) {
    failures.push(
      `${normalized}: hard-codes the control protocol version instead of using the generated contract`,
    );
  }
  if ([".rs", ".ts", ".tsx", ".css"].includes(extname(normalized))) {
    const lines = contents.split(/\r?\n/u).length;
    if (lines > 2000) failures.push(`${normalized}: ${lines} lines exceeds the 2000-line limit`);
  }
}

if (failures.length > 0) {
  process.stderr.write(`${failures.join("\n")}\n`);
  process.exit(1);
}
process.stdout.write(`Agent architecture check passed for ${listed.length} production files.\n`);
