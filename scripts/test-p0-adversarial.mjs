import { spawnSync } from "node:child_process";

const commands = [
  ["cargo", ["test", "--offline", "-p", "hachimi-storage", "side_effect"]],
  ["cargo", ["test", "--offline", "-p", "hachimi-agent", "cancellation"]],
  [
    "cargo",
    [
      "test",
      "--offline",
      "-p",
      "hachimi-agent",
      "late_success_after_dispatch_is_indeterminate_and_not_model_visible",
    ],
  ],
  ["cargo", ["test", "--offline", "-p", "hachimi-agent", "hostile_history"]],
  ["cargo", ["test", "--offline", "-p", "hachimi-agent", "prompt_injection"]],
  [
    "cargo",
    [
      "test",
      "--offline",
      "-p",
      "hachimi-workspace",
      "stale_generation_guard_fails_before_restricted_worker_dispatch",
    ],
  ],
  ["cargo", ["test", "--offline", "-p", "hachimi-sandbox", "final_spawn_rejects"]],
  ["cargo", ["test", "--offline", "-p", "hachimi-sandbox", "path_security"]],
  [
    "cargo",
    [
      "test",
      "--offline",
      "-p",
      "hachimi-sandbox",
      "--test",
      "windows_smoke",
      "windows_path_matrix_rejects_aliases_and_hard_links",
    ],
  ],
];

for (const [command, args] of commands) {
  const result = spawnSync(command, args, { stdio: "inherit", env: process.env });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
