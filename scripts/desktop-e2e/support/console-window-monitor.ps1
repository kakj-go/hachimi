param(
    [Parameter(Mandatory = $true)][string]$ApplicationPath,
    [Parameter(Mandatory = $true)][string]$StopFile,
    [Parameter(Mandatory = $true)][string]$ReportFile
)

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public sealed class HachimiWindowRecord {
    public long Handle { get; set; }
    public int ProcessId { get; set; }
    public string ClassName { get; set; }
    public bool Visible { get; set; }
}

public static class HachimiWindowProbe {
    private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr hwnd, StringBuilder className, int capacity);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);

    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr hwnd);

    public static List<HachimiWindowRecord> ListTopLevelWindows() {
        var windows = new List<HachimiWindowRecord>();
        EnumWindows((hwnd, _) => {
            var name = new StringBuilder(256);
            GetClassName(hwnd, name, name.Capacity);
            uint processId;
            GetWindowThreadProcessId(hwnd, out processId);
            windows.Add(new HachimiWindowRecord {
                Handle = hwnd.ToInt64(),
                ProcessId = unchecked((int)processId),
                ClassName = name.ToString(),
                Visible = IsWindowVisible(hwnd)
            });
            return true;
        }, IntPtr.Zero);
        return windows;
    }
}
'@

$resolvedApplication = [System.IO.Path]::GetFullPath($ApplicationPath)
$knownTreePids = [System.Collections.Generic.HashSet[int]]::new()
$seenHandles = [System.Collections.Generic.HashSet[long]]::new()
$findings = [System.Collections.Generic.List[object]]::new()

while (-not [System.IO.File]::Exists($StopFile)) {
    $processes = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, ExecutablePath, CommandLine)
    $processById = @{}
    foreach ($process in $processes) { $processById[[int]$process.ProcessId] = $process }
    $remaining = [System.Collections.Generic.List[object]]::new()
    foreach ($process in $processes) {
        $executable = [string]$process.ExecutablePath
        if ($executable -and [System.StringComparer]::OrdinalIgnoreCase.Equals(
                [System.IO.Path]::GetFullPath($executable),
                $resolvedApplication
            )) {
            [void]$knownTreePids.Add([int]$process.ProcessId)
        } else {
            $remaining.Add($process)
        }
    }

    for ($pass = 0; $pass -lt 8 -and $remaining.Count -gt 0; $pass++) {
        $next = [System.Collections.Generic.List[object]]::new()
        foreach ($process in $remaining) {
            if ($knownTreePids.Contains([int]$process.ParentProcessId)) {
                [void]$knownTreePids.Add([int]$process.ProcessId)
            } else {
                $next.Add($process)
            }
        }
        if ($next.Count -eq $remaining.Count) { break }
        $remaining = $next
    }

    foreach ($window in [HachimiWindowProbe]::ListTopLevelWindows()) {
        if ($window.ClassName -ne 'ConsoleWindowClass') { continue }
        if (-not $window.Visible) { continue }
        if (-not $knownTreePids.Contains($window.ProcessId)) { continue }
        if (-not $seenHandles.Add($window.Handle)) { continue }
        $owner = $processById[$window.ProcessId]
        $ownerName = [System.IO.Path]::GetFileName([string]$owner.ExecutablePath)
        if ($ownerName -in @('hachimi-desktop.exe', 'msedgewebview2.exe')) { continue }
        $findings.Add([pscustomobject]@{
                handle = $window.Handle
                processId = $window.ProcessId
                className = $window.ClassName
                executablePath = [string]$owner.ExecutablePath
                commandLine = [string]$owner.CommandLine
                observedAt = [DateTimeOffset]::UtcNow.ToString('O')
            })
    }
    [System.Threading.Thread]::Sleep(25)
}

$report = [pscustomobject]@{
    applicationPath = $resolvedApplication
    findings = @($findings)
}
[System.IO.File]::WriteAllText(
    $ReportFile,
    ($report | ConvertTo-Json -Depth 5),
    [System.Text.UTF8Encoding]::new($false)
)
