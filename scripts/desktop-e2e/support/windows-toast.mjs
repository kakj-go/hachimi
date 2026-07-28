import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const assertionScript = fileURLToPath(new URL("../../assert-windows-toast.ps1", import.meta.url));

export function assertWindowsToast(taskName, status, timeoutSeconds = 20) {
  if (process.platform !== "win32") {
    throw new Error("Windows toast UI Automation is only available on Windows.");
  }
  const result = spawnSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      assertionScript,
      "-TaskName",
      taskName,
      "-Status",
      status,
      "-TimeoutSeconds",
      String(timeoutSeconds),
    ],
    { encoding: "utf8", windowsHide: true },
  );
  if (result.status !== 0) {
    throw new Error(
      `Windows toast assertion failed: ${(result.stderr || result.stdout || "unknown error").trim()}`,
    );
  }
}
