import { spawnSync } from "node:child_process";

const cleanupScript = String.raw`
$ErrorActionPreference = "Stop"
$target = [IO.Path]::GetFullPath($env:HACHIMI_E2E_CLEANUP_EXE)
$keepNewest = [int]$env:HACHIMI_E2E_KEEP_NEWEST
$matches = @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
  try {
    $_.Path -and [IO.Path]::GetFullPath($_.Path).Equals(
      $target,
      [StringComparison]::OrdinalIgnoreCase
    )
  } catch {
    $false
  }
} | Sort-Object StartTime -Descending)
$matches | Select-Object -Skip $keepNewest | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 100
$remaining = @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
  try {
    $_.Path -and [IO.Path]::GetFullPath($_.Path).Equals(
      $target,
      [StringComparison]::OrdinalIgnoreCase
    )
  } catch {
    $false
  }
})
if ($remaining.Count -gt $keepNewest) {
  exit 1
}
`;

export function cleanupExecutableProcesses(executable, { keepNewest = 0 } = {}) {
  if (process.platform !== "win32" || !executable) return;
  const result = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", cleanupScript],
    {
      env: {
        ...process.env,
        HACHIMI_E2E_CLEANUP_EXE: executable,
        HACHIMI_E2E_KEEP_NEWEST: String(keepNewest),
      },
      encoding: "utf8",
      windowsHide: true,
    },
  );
  if (result.status !== 0) {
    throw new Error("Desktop E2E could not clean its exact application process set");
  }
}

export function terminateProcessTree(processId) {
  if (!Number.isInteger(processId) || processId <= 0) return;
  if (process.platform === "win32") {
    spawnSync("taskkill.exe", ["/PID", String(processId), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
    return;
  }
  try {
    process.kill(processId, "SIGTERM");
  } catch {
    // The child already exited.
  }
}
