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
$deadline = [DateTime]::UtcNow.AddSeconds(5)
do {
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
} while ($remaining.Count -gt $keepNewest -and [DateTime]::UtcNow -lt $deadline)
if ($remaining.Count -gt $keepNewest) {
  exit 1
}
`;

const cleanupGatewayScript = String.raw`
$ErrorActionPreference = "Stop"
$target = [IO.Path]::GetFullPath($env:HACHIMI_E2E_CLEANUP_EXE)
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$registered = (Get-ItemProperty -LiteralPath $runKey -Name "HachimiGateway" -ErrorAction SilentlyContinue).HachimiGateway
if ($registered) {
  $expected = ('"{0}" --gateway' -f $target)
  if ($registered.Equals($expected, [StringComparison]::OrdinalIgnoreCase)) {
    Remove-ItemProperty -LiteralPath $runKey -Name "HachimiGateway" -ErrorAction SilentlyContinue
  }
}
$matches = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
  $_.ExecutablePath -and $_.CommandLine -and
  [IO.Path]::GetFullPath($_.ExecutablePath).Equals(
    $target,
    [StringComparison]::OrdinalIgnoreCase
  ) -and
  $_.CommandLine.IndexOf("--gateway", [StringComparison]::OrdinalIgnoreCase) -ge 0
})
$matches | ForEach-Object {
  Stop-Process -Id ([int]$_.ProcessId) -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Milliseconds 100
$remaining = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
  $_.ExecutablePath -and $_.CommandLine -and
  [IO.Path]::GetFullPath($_.ExecutablePath).Equals(
    $target,
    [StringComparison]::OrdinalIgnoreCase
  ) -and
  $_.CommandLine.IndexOf("--gateway", [StringComparison]::OrdinalIgnoreCase) -ge 0
})
if ($remaining.Count -ne 0) {
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

export function cleanupGatewayProcesses(executable) {
  if (process.platform !== "win32" || !executable) return;
  const result = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", cleanupGatewayScript],
    {
      env: {
        ...process.env,
        HACHIMI_E2E_CLEANUP_EXE: executable,
      },
      encoding: "utf8",
      windowsHide: true,
    },
  );
  if (result.status !== 0) {
    throw new Error("Desktop E2E could not clean its exact per-user Gateway process set");
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
