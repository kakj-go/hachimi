param(
    [string]$HostExecutable = "target/manual-test/debug/cef-runtime/hachimi-cef-host.exe",
    [string]$ProfileDirectory = "target/manual-test/cef-ipc-smoke",
    [int]$ParentProcessId = 0,
    [string]$TlsTestUrl = "",
    [switch]$VerifyVisibleSurface
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$hostPath = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $HostExecutable))
$profilePath = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $ProfileDirectory))

if (-not (Test-Path -LiteralPath $hostPath -PathType Leaf)) {
    throw "CEF host does not exist: $hostPath"
}

if ($ParentProcessId -gt 0) {
    $parent = Get-Process -Id $ParentProcessId -ErrorAction Stop
    $parentHwnd = $parent.MainWindowHandle.ToInt64()
    $smokeWindow = $null
}
else {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Threading;
public sealed class HachimiCefSmokeWindow : IDisposable {
    private readonly ManualResetEventSlim ready = new ManualResetEventSlim(false);
    private readonly Thread thread;
    private uint threadId;
    public IntPtr Handle { get; private set; }
    public int Left { get; private set; }
    public int Top { get; private set; }
    public int PhysicalLeft { get; private set; }
    public int PhysicalTop { get; private set; }
    public int PhysicalWidth { get; private set; }
    public int PhysicalHeight { get; private set; }
    public uint Dpi { get; private set; }

    [StructLayout(LayoutKind.Sequential)]
    private struct Point { public int X; public int Y; }

    [StructLayout(LayoutKind.Sequential)]
    private struct Message {
        public IntPtr Window;
        public uint Id;
        public UIntPtr WParam;
        public IntPtr LParam;
        public uint Time;
        public Point Location;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { public int Left; public int Top; public int Right; public int Bottom; }

    [DllImport("kernel32.dll")]
    private static extern uint GetCurrentThreadId();

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr CreateWindowExW(
        uint extendedStyle,
        string className,
        string windowName,
        uint style,
        int x,
        int y,
        int width,
        int height,
        IntPtr parent,
        IntPtr menu,
        IntPtr instance,
        IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern bool DestroyWindow(IntPtr window);

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr window);

    [DllImport("user32.dll")]
    private static extern bool ShowWindow(IntPtr window, int command);

    [DllImport("user32.dll")]
    private static extern bool UpdateWindow(IntPtr window);

    [DllImport("user32.dll")]
    private static extern bool GetWindowRect(IntPtr window, out Rect rect);

    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr window);

    [DllImport("user32.dll")]
    private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);

    [DllImport("dwmapi.dll")]
    private static extern int DwmGetWindowAttribute(IntPtr window, int attribute, out Rect value, int size);

    [DllImport("user32.dll")]
    private static extern int GetMessageW(out Message message, IntPtr window, uint min, uint max);

    [DllImport("user32.dll")]
    private static extern bool TranslateMessage(ref Message message);

    [DllImport("user32.dll")]
    private static extern IntPtr DispatchMessageW(ref Message message);

    [DllImport("user32.dll")]
    private static extern bool PostThreadMessageW(uint threadId, uint message, UIntPtr wParam, IntPtr lParam);

    public HachimiCefSmokeWindow(bool visible) {
        PhysicalWidth = 800;
        PhysicalHeight = 600;
        Dpi = 96;
        thread = new Thread(() => {
            SetThreadDpiAwarenessContext(new IntPtr(-4));
            threadId = GetCurrentThreadId();
            uint style = visible ? 0x90000000 : 0x80000000;
            int x = visible ? 80 : -32000;
            int y = visible ? 80 : -32000;
            uint extendedStyle = visible ? 0x00000008u : 0u;
            Handle = CreateWindowExW(extendedStyle, "STATIC", "Hachimi CEF smoke parent", style,
                x, y, 800, 600, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
            if (visible) {
                ShowWindow(Handle, 5);
                UpdateWindow(Handle);
                SetForegroundWindow(Handle);
                Rect rect;
                if (GetWindowRect(Handle, out rect)) {
                    Left = rect.Left;
                    Top = rect.Top;
                }
                uint dpi = GetDpiForWindow(Handle);
                if (dpi > 0) Dpi = dpi;
                Rect physical;
                if (DwmGetWindowAttribute(Handle, 9, out physical, Marshal.SizeOf<Rect>()) == 0) {
                    PhysicalLeft = physical.Left;
                    PhysicalTop = physical.Top;
                    PhysicalWidth = physical.Right - physical.Left;
                    PhysicalHeight = physical.Bottom - physical.Top;
                } else {
                    PhysicalLeft = Left;
                    PhysicalTop = Top;
                }
            }
            ready.Set();
            Message message;
            while (GetMessageW(out message, IntPtr.Zero, 0, 0) > 0) {
                TranslateMessage(ref message);
                DispatchMessageW(ref message);
            }
            if (Handle != IntPtr.Zero) {
                DestroyWindow(Handle);
                Handle = IntPtr.Zero;
            }
        });
        thread.IsBackground = true;
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        if (!ready.Wait(TimeSpan.FromSeconds(10))) {
            throw new TimeoutException("Smoke parent window did not start");
        }
    }

    public void Dispose() {
        PostThreadMessageW(threadId, 0x0012, UIntPtr.Zero, IntPtr.Zero);
        thread.Join(TimeSpan.FromSeconds(10));
        ready.Dispose();
    }
}
"@
    $smokeWindow = [HachimiCefSmokeWindow]::new([bool]$VerifyVisibleSurface)
    $parentHwnd = $smokeWindow.Handle.ToInt64()
}
if ($parentHwnd -eq 0) {
    throw "CEF smoke test requires a valid parent HWND"
}

$start = [System.Diagnostics.ProcessStartInfo]::new()
$start.FileName = $hostPath
$start.WorkingDirectory = Split-Path -Parent $hostPath
$start.UseShellExecute = $false
$start.RedirectStandardInput = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$cefArguments = @(
    "--hachimi-parent-hwnd=$parentHwnd",
    "--hachimi-profile-dir=$profilePath",
    "--hachimi-log-file=$(Join-Path $profilePath 'cef.log')"
)
if ($cefArguments.Where({ $_.Contains('"') }).Count -gt 0) {
    throw "CEF smoke paths must not contain quotes"
}
$start.Arguments = '"' + ($cefArguments -join '" "') + '"'

$process = [System.Diagnostics.Process]::Start($start)
$messages = [System.Collections.Generic.List[string]]::new()

function Read-CefMessage([int]$TimeoutMs = 15000) {
    $task = $process.StandardOutput.ReadLineAsync()
    if (-not $task.Wait($TimeoutMs)) {
        $received = $messages -join [Environment]::NewLine
        throw "Timed out waiting for CEF host output. Received:`n$received"
    }
    if ($null -eq $task.Result) {
        $stderr = $process.StandardError.ReadToEnd()
        throw "CEF host output closed unexpectedly: $stderr"
    }
    $messages.Add($task.Result)
    return $task.Result | ConvertFrom-Json
}

function Send-CefCommand([long]$RequestId, [hashtable]$Command) {
    $envelope = @{
        protocolVersion = 1
        requestId = $RequestId
        command = $Command
    } | ConvertTo-Json -Compress -Depth 20
    $process.StandardInput.WriteLine($envelope)
    $process.StandardInput.Flush()
}

function Wait-CefMessage([scriptblock]$Predicate, [int]$MaximumMessages = 50) {
    for ($index = 0; $index -lt $MaximumMessages; $index++) {
        $message = Read-CefMessage
        if (& $Predicate $message) {
            return $message
        }
    }
    throw "CEF host did not emit the expected message"
}

try {
    $ready = Read-CefMessage
    if ($ready.kind -ne "ready" -or $ready.protocol_version -ne 1) {
        throw "CEF host did not emit a compatible ready message"
    }

    $surfaceWidth = if ($VerifyVisibleSurface) { $smokeWindow.PhysicalWidth } else { 800 }
    $surfaceHeight = if ($VerifyVisibleSurface) { $smokeWindow.PhysicalHeight } else { 600 }
    Send-CefCommand 1 @{
        kind = "create_tab"
        tab_id = "smoke-tab-1"
        url = "https://example.com/"
        bounds = @{ x = 0; y = 0; width = $surfaceWidth; height = $surfaceHeight }
        visible = [bool]$VerifyVisibleSurface
    }
    Wait-CefMessage { param($message) $message.kind -eq "response" -and $message.request_id -eq 1 } | Out-Null
    Send-CefCommand 10 @{ kind = "activate_tab"; tab_id = "smoke-tab-1" }
    $activated = Wait-CefMessage {
        param($message)
        $message.kind -eq "response" -and $message.request_id -eq 10
    }
    if ($null -ne $activated.result.Err) {
        throw "CEF tab was acknowledged before its native runtime was ready"
    }
    Wait-CefMessage {
        param($message)
            $message.kind -eq "event" -and
            $message.event.kind -eq "tab_state_changed" -and
            $message.event.state.url -like "https://example.com/*" -and
            $message.event.state.title -eq "Example Domain" -and
            -not $message.event.state.loading
    } | Out-Null

    Send-CefCommand 2 @{
        kind = "dev_tools"
        tab_id = "smoke-tab-1"
        method = "Runtime.evaluate"
        full_access = $false
        params = @{
            expression = "({title: document.title, url: location.href, text: document.body.innerText})"
            returnByValue = $true
        }
    }
    $evaluation = Wait-CefMessage {
        param($message)
        $message.kind -eq "response" -and $message.request_id -eq 2
    }
    $page = $evaluation.result.Ok.result.result.value
    if ($page.title -ne "Example Domain" -or $page.url -notlike "https://example.com/*") {
        $detail = $evaluation | ConvertTo-Json -Compress -Depth 20
        throw "CEF DevTools did not observe the loaded page: $detail"
    }

    $surfaceScreenshot = $null
    $sampledColors = $null
    if ($VerifyVisibleSurface) {
        if ($null -eq $smokeWindow) {
            throw "Visible surface verification requires the script-owned smoke parent HWND"
        }
        Add-Type -AssemblyName System.Drawing
        Start-Sleep -Milliseconds 750
        New-Item -ItemType Directory -Force -Path $profilePath | Out-Null
        $surfaceScreenshot = Join-Path $profilePath "cef-visible-smoke.png"
        $bitmap = [System.Drawing.Bitmap]::new(
            $smokeWindow.PhysicalWidth,
            $smokeWindow.PhysicalHeight
        )
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.CopyFromScreen(
                $smokeWindow.PhysicalLeft,
                $smokeWindow.PhysicalTop,
                0,
                0,
                [System.Drawing.Size]::new(
                    $smokeWindow.PhysicalWidth,
                    $smokeWindow.PhysicalHeight
                ),
                [System.Drawing.CopyPixelOperation]::SourceCopy
            )
            $colors = [System.Collections.Generic.HashSet[int]]::new()
            for ($y = 20; $y -lt ($smokeWindow.PhysicalHeight - 20); $y += 20) {
                for ($x = 20; $x -lt ($smokeWindow.PhysicalWidth - 20); $x += 20) {
                    [void]$colors.Add($bitmap.GetPixel($x, $y).ToArgb())
                }
            }
            $sampledColors = $colors.Count
            if ($sampledColors -lt 8) {
                throw "CEF native surface was blank or effectively single-color ($sampledColors sampled colors)"
            }
            $bitmap.Save($surfaceScreenshot, [System.Drawing.Imaging.ImageFormat]::Png)
        }
        finally {
            $graphics.Dispose()
            $bitmap.Dispose()
        }
    }

    Send-CefCommand 3 @{
        kind = "navigate"
        tab_id = "smoke-tab-1"
        url = "https://hachimi-browser-smoke.invalid/"
    }
    $networkFailure = Wait-CefMessage {
        param($message)
        $message.kind -eq "event" -and
            $message.event.kind -eq "tab_state_changed" -and
            $null -ne $message.event.state.navigationError -and
            $message.event.state.navigationError.failedUrl -like "*hachimi-browser-smoke.invalid*"
    }

    Send-CefCommand 4 @{
        kind = "navigate"
        tab_id = "smoke-tab-1"
        url = "https://example.com/hachimi-browser-404"
    }
    Wait-CefMessage {
        param($message)
        $message.kind -eq "event" -and
            $message.event.kind -eq "tab_state_changed" -and
            $message.event.state.url -eq "https://example.com/hachimi-browser-404" -and
            $message.event.state.title -eq "Example Domain" -and
            $null -eq $message.event.state.navigationError -and
            -not $message.event.state.loading
    } | Out-Null

    $tlsFailure = $null
    if ($TlsTestUrl) {
        Send-CefCommand 6 @{
            kind = "navigate"
            tab_id = "smoke-tab-1"
            url = $TlsTestUrl
        }
        $tlsFailure = Wait-CefMessage {
            param($message)
            $message.kind -eq "event" -and
                $message.event.kind -eq "tab_state_changed" -and
                $null -ne $message.event.state.navigationError -and
                $message.event.state.navigationError.kind -eq "tls"
        }
    }
    $tlsError = if ($null -ne $tlsFailure) {
        $tlsFailure.event.state.navigationError.description
    } else {
        $null
    }

    Send-CefCommand 7 @{ kind = "shutdown" }
    $process.StandardInput.Close()
    if (-not $process.WaitForExit(15000)) {
        throw "CEF host did not shut down within 15 seconds"
    }
    if ($process.ExitCode -ne 0) {
        throw "CEF host exited with code $($process.ExitCode)"
    }

    [pscustomobject]@{
        ChromiumVersion = $ready.chromium_version
        LoadedTitle = $page.title
        LoadedUrl = $page.url
        NetworkError = $networkFailure.event.state.navigationError.description
        Http404Preserved = $true
        TlsError = $tlsError
        SurfaceScreenshot = $surfaceScreenshot
        SampledColors = $sampledColors
        Messages = $messages.Count
    }
}
finally {
    if (-not $process.HasExited) {
        $process.Kill($true)
        $process.WaitForExit()
    }
    $process.Dispose()
    if ($null -ne $smokeWindow) {
        $smokeWindow.Dispose()
    }
}
