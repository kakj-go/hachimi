param(
    [switch]$SkipPortable,
    [string]$OtherNtfsRoot = $env:HACHIMI_SANDBOX_OTHER_NTFS_ROOT
)

$ErrorActionPreference = "Stop"

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath failed with exit code $LASTEXITCODE"
    }
}

if ($env:OS -ne "Windows_NT") {
    throw "The Windows release suite can only run on Windows."
}
$principal = [Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run pnpm test:windows:release from an elevated PowerShell session."
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$targetRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot "target"))
$runId = [DateTimeOffset]::UtcNow.ToString("yyyyMMddTHHmmssZ")
$reportRoot = [System.IO.Path]::GetFullPath(
    (Join-Path $targetRoot (Join-Path "windows-release-validation" $runId))
)
if (-not $reportRoot.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Resolved validation output is outside target."
}
New-Item -ItemType Directory -Path $reportRoot -Force | Out-Null
$attestationRoot = Join-Path $reportRoot "attestation"
New-Item -ItemType Directory -Path $attestationRoot -Force | Out-Null

Push-Location $repoRoot
try {
    Invoke-Checked "cargo" @(
        "build", "-p", "hachimi-sandbox", "--features", "windows-smoke", "--bins"
    )
    Invoke-Checked "cargo" @(
        "build", "-p", "hachimi-capabilities", "--bin", "hachimi-mcp-test-server"
    )
    Invoke-Checked "cargo" @(
        "build", "-p", "hachimi-workspace", "--bin", "hachimi-workspace-worker"
    )

    $debugRoot = Join-Path $targetRoot "debug"
    $hostBin = Join-Path $reportRoot "host-bin"
    New-Item -ItemType Directory -Path $hostBin -Force | Out-Null
    $binaryNames = @(
        "hachimi-sandbox-setup.exe",
        "hachimi-sandbox-launcher.exe",
        "hachimi-sandbox-canary.exe",
        "hachimi-sandbox-attest.exe",
        "hachimi-mcp-test-server.exe",
        "hachimi-workspace-worker.exe"
    )
    foreach ($name in $binaryNames) {
        $source = Join-Path $debugRoot $name
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Required release smoke binary is missing."
        }
        Copy-Item -LiteralPath $source -Destination (Join-Path $hostBin $name) -Force
    }
    $setup = Join-Path $hostBin "hachimi-sandbox-setup.exe"
    $launcher = Join-Path $hostBin "hachimi-sandbox-launcher.exe"
    $canary = Join-Path $hostBin "hachimi-sandbox-canary.exe"
    $attest = Join-Path $hostBin "hachimi-sandbox-attest.exe"
    $marker = Join-Path $reportRoot "sandbox\setup.json"
    Invoke-Checked $setup @("--marker", $marker, "--launcher", $launcher)
    $attestationJson = & $attest --marker $marker --launcher $launcher --canary $canary --root $attestationRoot
    $attestationJson | Set-Content -LiteralPath (Join-Path $reportRoot "attestation.json") -Encoding UTF8
    if ($LASTEXITCODE -ne 0) {
        throw "Initial Sandbox attestation failed: $attestationJson"
    }
    $attestation = $attestationJson | ConvertFrom-Json
    if (-not ($attestation.osEnforced -and $attestation.filesystemEnforced -and $attestation.processEnforced -and $attestation.networkEnforced)) {
        throw "Sandbox attestation did not enforce all four boundaries."
    }

    $env:HACHIMI_SANDBOX_MARKER = $marker
    $env:HACHIMI_SANDBOX_LAUNCHER = $launcher
    $env:HACHIMI_SANDBOX_CANARY = $canary
    $env:HACHIMI_SANDBOX_ATTESTATION_ROOT = $attestationRoot
    $env:HACHIMI_RELEASE_MCP_TEST_SERVER = Join-Path $hostBin "hachimi-mcp-test-server.exe"
    $env:HACHIMI_RELEASE_WORKSPACE_WORKER = Join-Path $hostBin "hachimi-workspace-worker.exe"
    if (-not [string]::IsNullOrWhiteSpace($OtherNtfsRoot)) {
        $resolvedOtherRoot = [System.IO.Path]::GetFullPath($OtherNtfsRoot)
        if (-not (Test-Path -LiteralPath $resolvedOtherRoot -PathType Container)) {
            throw "The alternate NTFS test root does not exist."
        }
        $env:HACHIMI_SANDBOX_OTHER_NTFS_ROOT = $resolvedOtherRoot
    }

    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-sandbox", "--features", "windows-smoke", "--", "--ignored", "--test-threads=1"
    )
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-capabilities", "--test", "mcp_stdio",
        "production_stdio_mcp_runs_restricted_and_cannot_connect_to_loopback",
        "--", "--ignored", "--exact", "--test-threads=1"
    )
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-workspace", "--test", "worker_process",
        "restricted_workspace_worker_executes_a_checkout_bound_write",
        "--", "--ignored", "--exact", "--test-threads=1"
    )
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-workspace", "--test", "worker_process",
        "restricted_workspace_worker_creates_an_empty_initial_commit_without_touching_index",
        "--", "--ignored", "--exact", "--test-threads=1"
    )
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-workspace", "--test", "worker_process",
        "restricted_agent_exec_tool_runs_through_policy_and_workspace_sandbox",
        "--", "--ignored", "--exact", "--test-threads=1"
    )
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-process",
        "terminal_conpty_uses_the_restricted_launcher_and_kills_its_process_tree",
        "--", "--ignored", "--test-threads=1"
    )
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-scheduler",
        "system_clock_at_every_and_six_field_cron_soak_without_duplicate_invocations",
        "--", "--ignored", "--test-threads=1"
    )
    $env:HACHIMI_DESKTOP_E2E_REAL_SANDBOX = "1"
    $env:HACHIMI_DESKTOP_E2E_ASSERT_TOAST = "1"
    Invoke-Checked "corepack" @("pnpm", "test:desktop:e2e")

    $portableAttested = $false
    if (-not $SkipPortable) {
        Invoke-Checked "corepack" @("pnpm", "build:portable")
        $portableRoot = Join-Path $targetRoot "portable\Hachimi"
        $portableMarker = Join-Path $portableRoot "data\sandbox\windows\setup.json"
        $portableSetup = Join-Path $portableRoot "hachimi-sandbox-setup.exe"
        $portableLauncher = Join-Path $portableRoot "hachimi-sandbox-launcher.exe"
        $portableCanary = Join-Path $portableRoot "hachimi-sandbox-canary.exe"
        $portableAttest = Join-Path $portableRoot "hachimi-sandbox-attest.exe"
        $portableRootProbe = Join-Path $reportRoot "portable-attestation"
        New-Item -ItemType Directory -Path $portableRootProbe -Force | Out-Null
        Invoke-Checked $portableSetup @(
            "--marker", $portableMarker, "--launcher", $portableLauncher
        )
        for ($attempt = 0; $attempt -lt 2; $attempt++) {
            Invoke-Checked $portableAttest @(
                "--marker", $portableMarker,
                "--launcher", $portableLauncher,
                "--canary", $portableCanary,
                "--root", $portableRootProbe
            )
        }
        $portableAttested = $true
    }

    [ordered]@{
        schemaVersion = 1
        completedAtUtc = [DateTimeOffset]::UtcNow.ToString("O")
        elevated = $true
        sandbox = [ordered]@{
            readiness = $attestation.readiness
            osEnforced = [bool]$attestation.osEnforced
            filesystemEnforced = [bool]$attestation.filesystemEnforced
            processEnforced = [bool]$attestation.processEnforced
            networkEnforced = [bool]$attestation.networkEnforced
            stableErrorCode = $attestation.stableErrorCode
        }
        sentinelHandle = "passed"
        workspaceWorker = "passed"
        initialCommit = "passed"
        agentExec = "passed"
        mcpStdio = "passed"
        terminalConPty = "passed"
        desktopE2e = "passed"
        systemNotificationUiAutomation = "passed"
        portableRestartAttestation = if ($portableAttested) { "passed" } else { "skipped" }
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $reportRoot "summary.json") -Encoding UTF8
    Write-Host "Windows release gate passed. Sanitized report: target/windows-release-validation/$runId/summary.json"
} finally {
    Pop-Location
}
