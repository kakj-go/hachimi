param(
    [switch]$SkipPortable,
    [switch]$SkipBuild,
    [string]$InstallerPath = "",
    [string]$MsiPath = "",
    [string]$PortablePath = "",
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

function Protect-SummaryText {
    param(
        [string]$Text,
        [string[]]$SensitivePaths = @()
    )
    $protected = [string]$Text
    foreach ($path in ($SensitivePaths | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Sort-Object Length -Descending -Unique)) {
        $protected = $protected -replace "(?i)$([Regex]::Escape($path))", "<redacted-path>"
    }
    $protected = $protected -replace '(?i)\b(token|secret|credential|password|api[_-]?key)\b(\s*[:=]\s*)["'']?[^\s,;"'']+', '$1$2<redacted>'
    if ($protected.Length -gt 2048) {
        return $protected.Substring(0, 2048) + "...<truncated>"
    }
    return $protected
}

function Get-TextSha256 {
    param([Parameter(Mandatory = $true)][string]$Text)
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($Text)
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        return -join ($sha256.ComputeHash($bytes) | ForEach-Object { $_.ToString("x2") })
    } finally {
        $sha256.Dispose()
    }
}

function Write-JsonUtf8NoBom {
    param(
        [Parameter(Mandatory = $true)][string]$LiteralPath,
        [Parameter(Mandatory = $true)][object]$Value,
        [int]$Depth = 8
    )
    $json = $Value | ConvertTo-Json -Depth $Depth
    [IO.File]::WriteAllText($LiteralPath, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
}

function Get-SourceRegistryDigests {
    param([Parameter(Mandatory = $true)][string]$Root)
    return [ordered]@{
        openai = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $Root "docs\references\openai\registry.json")).Hash.ToLowerInvariant()
        forge = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $Root "docs\references\forge\registry.json")).Hash.ToLowerInvariant()
        enterprise = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $Root "docs\references\enterprise\registry.json")).Hash.ToLowerInvariant()
        channels = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $Root "docs\references\channels\registry.json")).Hash.ToLowerInvariant()
    }
}

function Get-ReleaseArtifactDigests {
    param(
        [Parameter(Mandatory = $true)][string]$PrimaryInstaller,
        [AllowEmptyString()][string]$ConfiguredPaths
    )
    $paths = @($PrimaryInstaller)
    if (-not [string]::IsNullOrWhiteSpace($ConfiguredPaths)) {
        $paths += $ConfiguredPaths -split [System.IO.Path]::PathSeparator
    }
    return @($paths |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
        ForEach-Object { [System.IO.Path]::GetFullPath($_) } |
        Sort-Object -Unique |
        ForEach-Object {
            if (-not (Test-Path -LiteralPath $_ -PathType Leaf)) {
                throw "Release artifact is missing: $_"
            }
            [ordered]@{
                name = [System.IO.Path]::GetFileName($_)
                sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_).Hash.ToLowerInvariant()
            }
        })
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
$summaryPath = Join-Path $reportRoot "summary.json"
$summary = [ordered]@{
    schemaVersion = 1
    gateKind = "windows_elevated"
    status = "running"
    version = $null
    commitSha = (& git -C $repoRoot rev-parse HEAD).Trim()
    artifactSha256 = @()
    sourceRegistrySha256 = (Get-SourceRegistryDigests -Root $repoRoot)
    environmentFingerprint = (Get-TextSha256 -Text "$env:RUNNER_NAME|$env:OS|windows_elevated")
    checks = @()
    startedAtUtc = [DateTimeOffset]::UtcNow.ToString("O")
    completedAtUtc = $null
    elevated = $true
    candidateVersion = $null
    installerSha256 = $null
    sandbox = $null
    sentinelHandle = "not_run"
    linkedWorktreeAcl = "not_run"
    highPrivilegeWindowBoundary = "not_run"
    workspaceWorker = "not_run"
    initialCommit = "not_run"
    agentExec = "not_run"
    mcpStdio = "not_run"
    terminalConPty = "not_run"
    schedulerRealClockSoak = "not_run"
    desktopE2e = "not_run"
    systemNotificationUiAutomation = "not_run"
    portableRestartAttestation = "not_run"
    msiPackageLicenses = "not_run"
    portablePackageLicenses = "not_run"
    failure = $null
}

Push-Location $repoRoot
try {
    Invoke-Checked "corepack" @("pnpm", "release:check-clean")
    Invoke-Checked "corepack" @("pnpm", "runtime:prepare")
    if (-not $SkipBuild) {
        Invoke-Checked "corepack" @("pnpm", "build:installer")
    }
    $tauriConfig = Get-Content -LiteralPath (Join-Path $repoRoot "apps\desktop\src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json
    $summary.candidateVersion = [string]$tauriConfig.version
    $summary.version = $summary.candidateVersion
    if ([string]::IsNullOrWhiteSpace($summary.candidateVersion)) {
        throw "Tauri configuration does not declare a candidate version."
    }
    if ([string]::IsNullOrWhiteSpace($InstallerPath)) {
        $installerName = "Hachimi_{0}_x64-setup.exe" -f $summary.candidateVersion
        $InstallerPath = Join-Path $targetRoot (Join-Path "release\bundle\nsis" $installerName)
    }
    $InstallerPath = [System.IO.Path]::GetFullPath($InstallerPath)
    if (-not (Test-Path -LiteralPath $InstallerPath -PathType Leaf)) {
        throw "Candidate NSIS installer is missing."
    }
    $summary.installerSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $InstallerPath).Hash
    $summary.artifactSha256 = Get-ReleaseArtifactDigests -PrimaryInstaller $InstallerPath -ConfiguredPaths $env:HACHIMI_RELEASE_ARTIFACTS
    @(
        "candidateVersion=$($summary.candidateVersion)",
        "installerSha256=$($summary.installerSha256)"
    ) | Set-Content -LiteralPath (Join-Path $reportRoot "installer-sha256.txt") -Encoding UTF8

    if ([string]::IsNullOrWhiteSpace($MsiPath)) {
        if (-not [string]::IsNullOrWhiteSpace($env:HACHIMI_RELEASE_ARTIFACTS)) {
            $MsiPath = $env:HACHIMI_RELEASE_ARTIFACTS -split [System.IO.Path]::PathSeparator |
                Where-Object { [System.IO.Path]::GetExtension($_) -eq ".msi" } |
                Select-Object -First 1
        }
        if ([string]::IsNullOrWhiteSpace($MsiPath) -and -not $SkipBuild) {
            $MsiPath = Get-ChildItem -LiteralPath (Join-Path $targetRoot "release\bundle\msi") -Filter "*.msi" -File |
                Select-Object -First 1 -ExpandProperty FullName
        }
    }
    if ([string]::IsNullOrWhiteSpace($MsiPath)) {
        throw "Candidate MSI path is required for packaged-license verification."
    }
    $MsiPath = [System.IO.Path]::GetFullPath($MsiPath)
    if (-not (Test-Path -LiteralPath $MsiPath -PathType Leaf)) {
        throw "Candidate MSI is missing: $MsiPath"
    }
    $msiExtractRoot = Join-Path $reportRoot "msi-package"
    New-Item -ItemType Directory -Path $msiExtractRoot -Force | Out-Null
    Invoke-Checked "msiexec.exe" @("/a", $MsiPath, "/qn", "TARGETDIR=$msiExtractRoot")
    Invoke-Checked "powershell.exe" @(
        "-NoProfile", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $repoRoot "scripts\release\test-package-licenses.ps1"),
        "-PackageRoot", $msiExtractRoot
    )
    $summary.msiPackageLicenses = "passed"

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
    $summary.sandbox = [ordered]@{
        readiness = $attestation.readiness
        osEnforced = [bool]$attestation.osEnforced
        filesystemEnforced = [bool]$attestation.filesystemEnforced
        processEnforced = [bool]$attestation.processEnforced
        networkEnforced = [bool]$attestation.networkEnforced
        stableErrorCode = $attestation.stableErrorCode
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
    $summary.sentinelHandle = "passed"
    $summary.linkedWorktreeAcl = "passed"
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-computer", "elevated_and_hachimi_windows_are_rejected",
        "--", "--test-threads=1"
    )
    $summary.highPrivilegeWindowBoundary = "passed"
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-capabilities", "--test", "mcp_stdio",
        "production_stdio_mcp_runs_restricted_and_cannot_connect_to_loopback",
        "--", "--ignored", "--exact", "--test-threads=1"
    )
    $summary.mcpStdio = "passed"
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-workspace", "--test", "worker_process",
        "restricted_workspace_worker_executes_a_checkout_bound_write",
        "--", "--ignored", "--exact", "--test-threads=1"
    )
    $summary.workspaceWorker = "passed"
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-workspace", "--test", "worker_process",
        "restricted_workspace_worker_creates_an_empty_initial_commit_without_touching_index",
        "--", "--ignored", "--exact", "--test-threads=1"
    )
    $summary.initialCommit = "passed"
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-workspace", "--test", "worker_process",
        "restricted_agent_exec_tool_runs_through_policy_and_workspace_sandbox",
        "--", "--ignored", "--exact", "--test-threads=1"
    )
    $summary.agentExec = "passed"
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-process",
        "terminal_conpty_uses_the_restricted_launcher_and_kills_its_process_tree",
        "--", "--ignored", "--test-threads=1"
    )
    $summary.terminalConPty = "passed"
    Invoke-Checked "cargo" @(
        "test", "-p", "hachimi-scheduler",
        "system_clock_at_every_and_six_field_cron_soak_without_duplicate_invocations",
        "--", "--ignored", "--test-threads=1"
    )
    $summary.schedulerRealClockSoak = "passed"
    $env:HACHIMI_DESKTOP_E2E_REAL_SANDBOX = "1"
    $env:HACHIMI_DESKTOP_E2E_ASSERT_TOAST = "1"
    Invoke-Checked "corepack" @("pnpm", "test:desktop:e2e")
    $summary.desktopE2e = "passed"
    $summary.systemNotificationUiAutomation = "passed"

    $portableAttested = $false
    if (-not $SkipPortable) {
        if (-not $SkipBuild) {
            Invoke-Checked "corepack" @("pnpm", "build:portable")
            $portableRoot = Join-Path $targetRoot "portable\Hachimi"
        } else {
            if ([string]::IsNullOrWhiteSpace($PortablePath)) {
                throw "-PortablePath is required when -SkipBuild is used without -SkipPortable."
            }
            $PortablePath = [System.IO.Path]::GetFullPath($PortablePath)
            if (-not (Test-Path -LiteralPath $PortablePath -PathType Leaf)) {
                throw "Candidate portable ZIP is missing: $PortablePath"
            }
            $portableExtractRoot = Join-Path $reportRoot "portable-package"
            Expand-Archive -LiteralPath $PortablePath -DestinationPath $portableExtractRoot -Force
            $portableRoot = Join-Path $portableExtractRoot "Hachimi"
        }
        Invoke-Checked "powershell.exe" @(
            "-NoProfile", "-ExecutionPolicy", "Bypass",
            "-File", (Join-Path $repoRoot "scripts\release\test-package-licenses.ps1"),
            "-PackageRoot", $portableRoot
        )
        $summary.portablePackageLicenses = "passed"
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
    $summary.portableRestartAttestation = if ($portableAttested) { "passed" } else { "skipped" }
    $summary.checks = @(
        [ordered]@{ id = "windows_elevated_release_suite"; status = "passed"; detailsHash = (Get-TextSha256 -Text "windows_elevated_release_suite:passed") },
        [ordered]@{ id = "portable_restart_attestation"; status = $summary.portableRestartAttestation; detailsHash = (Get-TextSha256 -Text "portable_restart_attestation:$($summary.portableRestartAttestation)") },
        [ordered]@{ id = "msi_package_licenses"; status = $summary.msiPackageLicenses; detailsHash = (Get-TextSha256 -Text "msi_package_licenses:$($summary.msiPackageLicenses)") },
        [ordered]@{ id = "portable_package_licenses"; status = $summary.portablePackageLicenses; detailsHash = (Get-TextSha256 -Text "portable_package_licenses:$($summary.portablePackageLicenses)") }
    )
    $summary.status = "passed"
    $summary.completedAtUtc = [DateTimeOffset]::UtcNow.ToString("O")
    Write-JsonUtf8NoBom -LiteralPath $summaryPath -Value $summary -Depth 8
    @(
        "status=passed",
        "candidateVersion=$($summary.candidateVersion)",
        "installerSha256=$($summary.installerSha256)",
        "sandbox=$($summary.sandbox.readiness)",
        "desktopE2e=$($summary.desktopE2e)",
        "failure=null"
    ) | Set-Content -LiteralPath (Join-Path $reportRoot "gate.sanitized.log") -Encoding UTF8
    Write-Host "Windows release gate passed. Sanitized report: target/windows-release-validation/$runId/summary.json"
} catch {
    $summary.status = "failed"
    $summary.completedAtUtc = [DateTimeOffset]::UtcNow.ToString("O")
    $summary.failure = [ordered]@{
        code = "windows_elevated_gate_failed"
        message = Protect-SummaryText -Text $_.Exception.Message -SensitivePaths @(
            $repoRoot,
            $targetRoot,
            $reportRoot,
            $InstallerPath,
            $MsiPath,
            $PortablePath,
            $OtherNtfsRoot,
            $env:USERPROFILE
        )
    }
    $summary.checks = @([ordered]@{
        id = "windows_elevated_release_suite"
        status = "failed"
        detailsHash = (Get-TextSha256 -Text $summary.failure.message)
    })
    Write-JsonUtf8NoBom -LiteralPath $summaryPath -Value $summary -Depth 8
    @(
        "status=failed",
        "candidateVersion=$($summary.candidateVersion)",
        "installerSha256=$($summary.installerSha256)",
        "failure=$($summary.failure.message)"
    ) | Set-Content -LiteralPath (Join-Path $reportRoot "gate.sanitized.log") -Encoding UTF8
    throw
} finally {
    Pop-Location
}
