param(
    [string]$InstallerPath = "",
    [string]$PreviousInstallerPath = "",
    [string]$ExpectedPreviousVersion = "0.2.0",
    [switch]$SkipBuild,
    [switch]$SkipDesktopE2E
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

function Get-ValidSandboxMarker {
    param(
        [Parameter(Mandatory = $true)][string]$LiteralPath
    )
    if (-not (Test-Path -LiteralPath $LiteralPath -PathType Leaf)) {
        return $null
    }
    try {
        $marker = Get-Content -LiteralPath $LiteralPath -Raw | ConvertFrom-Json
        if ($marker.version -and $marker.appContainerSid) {
            return $marker
        }
    } catch {
        # Startup may be replacing the marker atomically. Keep polling until the
        # complete document is visible instead of treating the old file as ready.
    }
    return $null
}

function Start-HachimiProbe {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string]$DataRoot,
        [Parameter(Mandatory = $true)][string]$Marker,
        [string]$ExpectedRuntimePath = "",
        [string]$ExpectedRuntimeSha256 = "",
        [string]$RejectedAppContainerSid = "",
        [int]$TimeoutSeconds = 45
    )
    $env:HACHIMI_DATA_DIR = $DataRoot
    $process = Start-Process -FilePath $Executable -PassThru -WindowStyle Hidden
    try {
        $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
        while ([DateTimeOffset]::UtcNow -lt $deadline) {
            $validMarker = Get-ValidSandboxMarker -LiteralPath $Marker
            $runtimeReady = $true
            if (-not [string]::IsNullOrWhiteSpace($ExpectedRuntimePath)) {
                $runtimeReady = (Test-Path -LiteralPath $ExpectedRuntimePath -PathType Leaf) -and
                    ((Get-FileHash -Algorithm SHA256 -LiteralPath $ExpectedRuntimePath).Hash -eq $ExpectedRuntimeSha256)
            }
            $profileReady = $true
            if (-not [string]::IsNullOrWhiteSpace($RejectedAppContainerSid)) {
                $profileReady = $null -ne $validMarker -and
                    $validMarker.appContainerSid -ne $RejectedAppContainerSid
            }
            if ($null -ne $validMarker -and $runtimeReady -and $profileReady) {
                return $validMarker
            }
            if ($process.HasExited) {
                throw "Installed Hachimi exited before the requested Sandbox state was ready (exit code $($process.ExitCode))."
            }
            Start-Sleep -Milliseconds 250
        }
        throw "Timed out waiting for a valid, repaired Sandbox state."
    } finally {
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force
            $process.WaitForExit(10000) | Out-Null
        }
    }
}

function Protect-SummaryText {
    param(
        [AllowNull()][string]$Text,
        [AllowEmptyCollection()][object[]]$SensitivePaths = @()
    )
    if ([string]::IsNullOrEmpty($Text)) {
        return $Text
    }
    $protected = $Text
    $paths = $SensitivePaths |
        Where-Object { $_ -is [string] -and -not [string]::IsNullOrWhiteSpace($_) } |
        ForEach-Object {
            $candidatePath = $_
            try { [System.IO.Path]::GetFullPath($candidatePath) } catch { $candidatePath }
        } |
        Sort-Object Length -Descending -Unique
    foreach ($path in $paths) {
        $protected = $protected -replace "(?i)$([Regex]::Escape($path))", "<redacted-path>"
    }
    $protected = $protected -replace '(?i)(authorization\s*:\s*bearer\s+)[^\s,;]+', '$1<redacted>'
    $protected = $protected -replace '(?i)\b(bearer)\s+[A-Za-z0-9._~+/=-]+', '$1 <redacted>'
    $protected = $protected -replace '(?i)\b(token|secret|credential|password|api[_-]?key)\b(\s*[:=]\s*)["'']?[^\s,;"'']+', '$1$2<redacted>'
    if ($protected.Length -gt 2048) {
        $protected = $protected.Substring(0, 2048) + "...<truncated>"
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
    throw "The standard-user Windows release gate can only run on Windows."
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$targetRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot "target"))
$runId = [DateTimeOffset]::UtcNow.ToString("yyyyMMddTHHmmssZ")
$reportRoot = [System.IO.Path]::GetFullPath(
    (Join-Path $targetRoot (Join-Path "windows-standard-user-validation" $runId))
)
if (-not $reportRoot.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Resolved validation root escaped target."
}
New-Item -ItemType Directory -Path $reportRoot -Force | Out-Null
$summaryPath = Join-Path $reportRoot "summary.json"
$summary = [ordered]@{
    schemaVersion = 1
    gateKind = "windows_standard_user"
    status = "running"
    version = $null
    commitSha = (& git -C $repoRoot rev-parse HEAD).Trim()
    artifactSha256 = @()
    sourceRegistrySha256 = (Get-SourceRegistryDigests -Root $repoRoot)
    environmentFingerprint = (Get-TextSha256 -Text "$env:RUNNER_NAME|$env:OS|windows_standard_user")
    checks = @()
    startedAtUtc = [DateTimeOffset]::UtcNow.ToString("O")
    completedAtUtc = $null
    accountSidSha256 = $null
    administratorsMember = $false
    elevated = $false
    installerSha256 = $null
    previousInstallerSha256 = $null
    previousVersion = $null
    candidateVersion = $null
    installRoot = $null
    dataRoot = $null
    firstBootstrap = "not_run"
    markerRepair = "not_run"
    runtimeRepair = "not_run"
    profileSidRepair = "not_run"
    crossVersionUpgrade = "not_run"
    initialMarkerHash = $null
    repairedMarkerHash = $null
    attestation = $null
    localHostDeterministicTests = "not_run"
    browserDocumentResourcePolicy = "not_run"
    managedBrowserFileTransfer = "not_run"
    chromeExtensionFileTransfer = "not_run"
    pluginHookRuntime = "not_run"
    pluginAssetCustomUi = "not_run"
    connectorSidecar = "not_run"
    channelSidecar = "not_run"
    scheduledHost = "not_run"
    workspaceWorker = "not_run"
    workspaceGit = "not_run"
    agentExec = "not_run"
    terminalConPty = "not_run"
    restrictedStdioMcp = "not_run"
    schedulerRealClockSoak = "not_run"
    managedChromiumInteractiveSmoke = "not_run"
    computerNotepadInteractiveSmoke = "not_run"
    gatewayPerUserStartupSmoke = "not_run"
    skipDesktopE2E = [bool]$SkipDesktopE2E
    desktopE2ESkipped = [bool]$SkipDesktopE2E
    desktopE2e = "not_run"
    packageLicenses = "not_run"
    logsPath = $null
    failure = $null
}

Push-Location $repoRoot
try {
    Invoke-Checked "corepack" @("pnpm", "release:check-clean")
    if ($SkipDesktopE2E -and $env:GITHUB_ACTIONS -eq "true") {
        throw "The release standard-user Job cannot skip Desktop E2E."
    }
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $summary.accountSidSha256 = Get-TextSha256 -Text $identity.User.Value
    $administratorSid = "S-1-5-32-544"
    $tokenGroupsCsv = & whoami.exe /groups /fo csv /nh
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to inspect the current account group memberships."
    }
    $tokenGroups = $tokenGroupsCsv | ConvertFrom-Csv -Header GroupName, Type, Sid, Attributes
    $isAdministratorMember = $tokenGroups | Where-Object { $_.Sid -eq $administratorSid }
    $summary.administratorsMember = [bool]$isAdministratorMember
    if ($isAdministratorMember) {
        throw "This gate must run from an account that is not a member of BUILTIN\Administrators."
    }

    $principal = [Security.Principal.WindowsPrincipal]$identity
    $isElevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    $summary.elevated = $isElevated
    if ($isElevated) {
        throw "This gate must not run with an elevated token."
    }

    if (-not $SkipBuild) {
        Invoke-Checked "corepack" @("pnpm", "runtime:prepare")
        Invoke-Checked "corepack" @("pnpm", "build:installer")
    }
    $tauriConfigPath = Join-Path $repoRoot "apps\desktop\src-tauri\tauri.conf.json"
    $tauriConfig = Get-Content -LiteralPath $tauriConfigPath -Raw | ConvertFrom-Json
    $expectedCandidateVersion = [string]$tauriConfig.version
    if ([string]::IsNullOrWhiteSpace($expectedCandidateVersion)) {
        throw "Tauri configuration does not declare a candidate version."
    }
    if ([string]::IsNullOrWhiteSpace($InstallerPath)) {
        $candidateInstallerName = "Hachimi_{0}_x64-setup.exe" -f $expectedCandidateVersion
        $InstallerPath = Join-Path $targetRoot (Join-Path "release\bundle\nsis" $candidateInstallerName)
    }
    $InstallerPath = [System.IO.Path]::GetFullPath($InstallerPath)
    if (-not (Test-Path -LiteralPath $InstallerPath -PathType Leaf)) {
        throw "NSIS installer not found: $InstallerPath"
    }
    if ([string]::IsNullOrWhiteSpace($PreviousInstallerPath)) {
        $PreviousInstallerPath = $env:HACHIMI_PREVIOUS_INSTALLER
    }
    if ([string]::IsNullOrWhiteSpace($PreviousInstallerPath)) {
        throw "-PreviousInstallerPath or HACHIMI_PREVIOUS_INSTALLER is required; a same-version reinstall is not a cross-version upgrade."
    }
    $PreviousInstallerPath = [System.IO.Path]::GetFullPath($PreviousInstallerPath)
    if (-not (Test-Path -LiteralPath $PreviousInstallerPath -PathType Leaf)) {
        throw "Previous-version NSIS installer not found: $PreviousInstallerPath"
    }
    $summary.installerSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $InstallerPath).Hash
    $summary.version = $expectedCandidateVersion
    $summary.artifactSha256 = Get-ReleaseArtifactDigests -PrimaryInstaller $InstallerPath -ConfiguredPaths $env:HACHIMI_RELEASE_ARTIFACTS
    $summary.previousInstallerSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $PreviousInstallerPath).Hash
    if ($summary.installerSha256 -eq $summary.previousInstallerSha256) {
        throw "Previous and candidate installer hashes are identical; cross-version evidence is required."
    }
    @(
        "previousVersion=$ExpectedPreviousVersion",
        "previousInstallerSha256=$($summary.previousInstallerSha256)",
        "candidateVersion=$expectedCandidateVersion",
        "installerSha256=$($summary.installerSha256)"
    ) | Set-Content -LiteralPath (Join-Path $reportRoot "installer-sha256.txt") -Encoding UTF8

    # A standard account cannot approve UAC, so successful silent installation
    # proves the current-user package path does not require elevation.
    $install = Start-Process -FilePath $InstallerPath -ArgumentList "/S" -Wait -PassThru -WindowStyle Hidden
    if ($install.ExitCode -ne 0) {
        throw "Per-user NSIS install failed with code $($install.ExitCode)"
    }

    $uninstall = Get-ChildItem "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall" |
        ForEach-Object { Get-ItemProperty $_.PSPath } |
        Where-Object { $_.DisplayName -eq "Hachimi" } |
        Select-Object -First 1
    if (-not $uninstall -or [string]::IsNullOrWhiteSpace($uninstall.InstallLocation)) {
        throw "Hachimi current-user uninstall registration was not found."
    }
    $installRoot = [System.IO.Path]::GetFullPath($uninstall.InstallLocation)
    $summary.installRoot = "<per-user-install-root>"
    $summary.candidateVersion = $uninstall.DisplayVersion
    if ($summary.candidateVersion -ne $expectedCandidateVersion) {
        throw "Installed candidate version '$($summary.candidateVersion)' does not match Tauri version '$expectedCandidateVersion'."
    }
    $executable = @("Hachimi.exe", "hachimi-desktop.exe") |
        ForEach-Object { Join-Path $installRoot $_ } |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
        Select-Object -First 1
    if ([string]::IsNullOrWhiteSpace($executable)) {
        throw "Installed executable is missing: $executable"
    }
    Invoke-Checked "powershell.exe" @(
        "-NoProfile", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $repoRoot "scripts\release\test-package-licenses.ps1"),
        "-PackageRoot", $installRoot
    )
    $summary.packageLicenses = "passed"

    $dataRoot = Join-Path $reportRoot "data"
    $summary.dataRoot = "<per-user-data-root>"
    $summary.logsPath = "<per-user-data-root>/logs"
    $marker = Join-Path $dataRoot "sandbox\windows\setup.json"
    Start-HachimiProbe -Executable $executable -DataRoot $dataRoot -Marker $marker | Out-Null
    $summary.firstBootstrap = "passed"
    $initialMarkerHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $marker).Hash

    # Damage only the isolated validation marker, then require startup repair.
    "{}" | Set-Content -LiteralPath $marker -Encoding UTF8
    Start-HachimiProbe -Executable $executable -DataRoot $dataRoot -Marker $marker | Out-Null
    $repairedMarkerHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $marker).Hash
    $repairedMarker = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
    if (-not $repairedMarker.version -or -not $repairedMarker.appContainerSid) {
        throw "Sandbox marker was not repaired."
    }
    $summary.markerRepair = "passed"

    $runtimeRoot = Join-Path $dataRoot (Join-Path "sandbox\windows\runtime" $repairedMarker.version)
    $launcher = Join-Path $runtimeRoot "hachimi-sandbox-launcher.exe"
    $canary = Join-Path $runtimeRoot "hachimi-sandbox-canary.exe"
    $attester = Join-Path $runtimeRoot "hachimi-sandbox-attest.exe"
    foreach ($required in @($launcher, $canary, $attester)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
            throw "Managed Sandbox Runtime file is missing: $required"
        }
    }
    $launcherHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $launcher).Hash
    [IO.File]::WriteAllBytes($launcher, [byte[]](0x48, 0x41, 0x43, 0x48, 0x49, 0x4D, 0x49))
    Start-HachimiProbe `
        -Executable $executable `
        -DataRoot $dataRoot `
        -Marker $marker `
        -ExpectedRuntimePath $launcher `
        -ExpectedRuntimeSha256 $launcherHash | Out-Null
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $launcher).Hash -ne $launcherHash) {
        throw "Managed Sandbox Runtime binary was not atomically restaged."
    }
    $summary.runtimeRepair = "passed"

    $tamperedMarker = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
    $tamperedMarker.appContainerSid = "S-1-15-2-1"
    [IO.File]::WriteAllText(
        $marker,
        ($tamperedMarker | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )
    Start-HachimiProbe `
        -Executable $executable `
        -DataRoot $dataRoot `
        -Marker $marker `
        -RejectedAppContainerSid "S-1-15-2-1" | Out-Null
    $profileRepaired = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
    if ($profileRepaired.appContainerSid -eq "S-1-15-2-1") {
        throw "Sandbox Profile SID drift was not repaired."
    }
    $summary.profileSidRepair = "passed"

    $attestationOutput = & $attester --marker $marker --launcher $launcher --canary $canary --root (Join-Path $dataRoot "sandbox\windows\attestation")
    if ($LASTEXITCODE -ne 0) {
        throw "Sandbox attestation helper rejected the repaired Runtime."
    }
    $attestation = $attestationOutput | ConvertFrom-Json
    if ($attestation.readiness -ne "ready" -or
        -not $attestation.osEnforced -or
        -not $attestation.filesystemEnforced -or
        -not $attestation.processEnforced -or
        -not $attestation.networkEnforced) {
        throw "Sandbox attestation did not prove all four enforcement dimensions."
    }
    $summary.attestation = $attestation
    $env:HACHIMI_SANDBOX_MARKER = $marker
    $env:HACHIMI_SANDBOX_LAUNCHER = $launcher
    $env:HACHIMI_SANDBOX_CANARY = $canary
    $env:HACHIMI_SANDBOX_ATTESTATION_ROOT = Join-Path $dataRoot "sandbox\windows\attestation"

    $uninstaller = Get-ChildItem -LiteralPath $installRoot -Filter "uninstall*.exe" -File |
        Select-Object -First 1
    if (-not $uninstaller) {
        throw "Per-user uninstaller was not found before cross-version validation."
    }
    $removeCandidate = Start-Process -FilePath $uninstaller.FullName -ArgumentList "/S" -Wait -PassThru -WindowStyle Hidden
    if ($removeCandidate.ExitCode -ne 0) {
        throw "Candidate clean-install uninstall failed with code $($removeCandidate.ExitCode)"
    }
    $previousInstall = Start-Process -FilePath $PreviousInstallerPath -ArgumentList "/S" -Wait -PassThru -WindowStyle Hidden
    if ($previousInstall.ExitCode -ne 0) {
        throw "Previous-version per-user install failed with code $($previousInstall.ExitCode)"
    }
    $previousRegistration = Get-ChildItem "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall" |
        ForEach-Object { Get-ItemProperty $_.PSPath } |
        Where-Object { $_.DisplayName -eq "Hachimi" } |
        Select-Object -First 1
    if (-not $previousRegistration) {
        throw "Previous-version current-user registration was not found."
    }
    $summary.previousVersion = $previousRegistration.DisplayVersion
    if ($summary.previousVersion -ne $ExpectedPreviousVersion) {
        throw "Installed previous version '$($summary.previousVersion)' does not match expected baseline '$ExpectedPreviousVersion'."
    }
    $upgrade = Start-Process -FilePath $InstallerPath -ArgumentList "/S" -Wait -PassThru -WindowStyle Hidden
    if ($upgrade.ExitCode -ne 0) {
        throw "Per-user NSIS upgrade/reinstall failed with code $($upgrade.ExitCode)"
    }
    $candidateRegistration = Get-ChildItem "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall" |
        ForEach-Object { Get-ItemProperty $_.PSPath } |
        Where-Object { $_.DisplayName -eq "Hachimi" } |
        Select-Object -First 1
    if (-not $candidateRegistration -or $candidateRegistration.DisplayVersion -eq $summary.previousVersion) {
        throw "Candidate did not replace a distinct previous package version."
    }
    $summary.candidateVersion = $candidateRegistration.DisplayVersion
    if ($summary.candidateVersion -ne $expectedCandidateVersion) {
        throw "Upgraded candidate version '$($summary.candidateVersion)' does not match Tauri version '$expectedCandidateVersion'."
    }
    $summary.crossVersionUpgrade = "passed"
    Start-HachimiProbe -Executable $executable -DataRoot $dataRoot -Marker $marker | Out-Null

    $managedChromium = Get-ChildItem -LiteralPath $installRoot -Recurse -Filter "chrome.exe" -File |
        Where-Object { $_.FullName -match "managed-chromium" } |
        Select-Object -First 1
    if (-not $managedChromium) {
        throw "Installed managed Chromium runtime was not found."
    }
    $env:HACHIMI_MANAGED_CHROMIUM = $managedChromium.FullName
    $workspaceWorker = Get-ChildItem -LiteralPath $installRoot -Recurse -File |
        Where-Object { $_.Name -match "^hachimi-workspace-worker(\.exe|-x86_64-pc-windows-msvc\.exe)$" } |
        Select-Object -First 1
    if ($workspaceWorker) {
        $env:HACHIMI_RELEASE_WORKSPACE_WORKER = $workspaceWorker.FullName
    }

    Invoke-Checked "cargo" @("test", "-p", "hachimi-browser", "-p", "hachimi-computer", "-p", "hachimi-extensions", "-p", "hachimi-gateway")
    $summary.localHostDeterministicTests = "passed"
    $summary.browserDocumentResourcePolicy = "passed"
    Invoke-Checked "cargo" @("test", "-p", "hachimi-extensions", "--test", "hook_runtime", "--", "--nocapture", "--test-threads=1")
    $summary.pluginHookRuntime = "passed"
    Invoke-Checked "cargo" @("test", "-p", "hachimi-extensions", "--test", "sidecar_registry", "stdio_connector_executes_every_method_and_keeps_credentials_out_of_process_metadata", "--", "--exact", "--nocapture", "--test-threads=1")
    $summary.connectorSidecar = "passed"
    Invoke-Checked "cargo" @("test", "-p", "hachimi-gateway", "--test", "sidecar_provider", "channel_sidecar_executes_full_lifecycle_and_passes_transport_secret_only_on_stdin", "--", "--exact", "--nocapture", "--test-threads=1")
    $summary.channelSidecar = "passed"
    Invoke-Checked "cargo" @(
        "test",
        "-p",
        "hachimi-browser",
        "broker::tests::managed_chromium_observes_uploads_and_downloads_a_real_page",
        "--",
        "--ignored",
        "--exact",
        "--nocapture",
        "--test-threads=1"
    )
    $summary.managedChromiumInteractiveSmoke = "passed"
    $summary.managedBrowserFileTransfer = "passed"
    Invoke-Checked "cargo" @("test", "-p", "hachimi-workspace", "--test", "worker_process", "restricted_workspace_worker_executes_a_checkout_bound_write", "--", "--ignored", "--exact", "--nocapture", "--test-threads=1")
    $summary.workspaceWorker = "passed"
    Invoke-Checked "cargo" @("test", "-p", "hachimi-workspace", "--test", "worker_process", "restricted_workspace_worker_creates_an_empty_initial_commit_without_touching_index", "--", "--ignored", "--exact", "--nocapture", "--test-threads=1")
    $summary.workspaceGit = "passed"
    Invoke-Checked "cargo" @("test", "-p", "hachimi-workspace", "--test", "worker_process", "restricted_agent_exec_tool_runs_through_policy_and_workspace_sandbox", "--", "--ignored", "--exact", "--nocapture", "--test-threads=1")
    $summary.agentExec = "passed"
    Invoke-Checked "cargo" @("test", "-p", "hachimi-process", "tests::terminal_conpty_uses_the_restricted_launcher_and_kills_its_process_tree", "--", "--ignored", "--exact", "--nocapture", "--test-threads=1")
    $summary.terminalConPty = "passed"
    Invoke-Checked "cargo" @("test", "-p", "hachimi-capabilities", "--test", "mcp_stdio", "production_stdio_mcp_runs_restricted_and_cannot_connect_to_loopback", "--", "--ignored", "--exact", "--nocapture", "--test-threads=1")
    $summary.restrictedStdioMcp = "passed"
    Invoke-Checked "cargo" @("test", "-p", "hachimi-scheduler", "service::tests::system_clock_at_every_and_six_field_cron_soak_without_duplicate_invocations", "--", "--ignored", "--exact", "--nocapture", "--test-threads=1")
    $summary.schedulerRealClockSoak = "passed"
    $env:HACHIMI_STANDARD_USER_HACHIMI_EXE = $executable
    Invoke-Checked "cargo" @(
        "test",
        "-p",
        "hachimi-gateway",
        "tests::windows_per_user_startup_registration_roundtrips",
        "--",
        "--ignored",
        "--exact",
        "--nocapture"
    )
    $summary.gatewayPerUserStartupSmoke = "passed"
    Invoke-Checked "cargo" @(
        "test",
        "-p",
        "hachimi-computer",
        "platform::windows::tests::captures_and_controls_a_real_notepad_window_with_wgc",
        "--",
        "--ignored",
        "--exact",
        "--nocapture"
    )
    $summary.computerNotepadInteractiveSmoke = "passed"
    if (-not $SkipDesktopE2E) {
        $env:HACHIMI_DESKTOP_E2E_REAL_SANDBOX = "1"
        Invoke-Checked "corepack" @("pnpm", "test:desktop:e2e")
        $summary.desktopE2e = "passed"
        $summary.chromeExtensionFileTransfer = "passed"
        $summary.pluginAssetCustomUi = "passed"
        $summary.scheduledHost = "passed"
    } else {
        $summary.desktopE2e = "skipped"
    }

    $summary.status = "passed"
    $summary.completedAtUtc = [DateTimeOffset]::UtcNow.ToString("O")
    $summary.checks = @(
        [ordered]@{ id = "windows_standard_user_release_suite"; status = "passed"; detailsHash = (Get-TextSha256 -Text "windows_standard_user_release_suite:passed") },
        [ordered]@{ id = "desktop_e2e"; status = $summary.desktopE2e; detailsHash = (Get-TextSha256 -Text "desktop_e2e:$($summary.desktopE2e)") },
        [ordered]@{ id = "nsis_package_licenses"; status = $summary.packageLicenses; detailsHash = (Get-TextSha256 -Text "nsis_package_licenses:$($summary.packageLicenses)") }
    )
    $summary.initialMarkerHash = $initialMarkerHash
    $summary.repairedMarkerHash = $repairedMarkerHash
    Write-JsonUtf8NoBom -LiteralPath $summaryPath -Value $summary -Depth 8
    @(
        "status=passed",
        "administratorsMember=$($summary.administratorsMember)",
        "elevated=$($summary.elevated)",
        "previousVersion=$($summary.previousVersion)",
        "candidateVersion=$($summary.candidateVersion)",
        "previousInstallerSha256=$($summary.previousInstallerSha256)",
        "installerSha256=$($summary.installerSha256)",
        "desktopE2ESkipped=$($summary.desktopE2ESkipped)",
        "failure=null"
    ) | Set-Content -LiteralPath (Join-Path $reportRoot "gate.sanitized.log") -Encoding UTF8
    Write-Host "Standard-user Windows gate passed: target/windows-standard-user-validation/$runId/summary.json"
} catch {
    $summary.status = "failed"
    $summary.completedAtUtc = [DateTimeOffset]::UtcNow.ToString("O")
    $sensitivePaths = @(
        $repoRoot,
        $targetRoot,
        $reportRoot,
        $installRoot,
        $dataRoot,
        $InstallerPath,
        $PreviousInstallerPath,
        $env:USERPROFILE
    )
    $summary.failure = [ordered]@{
        code = "windows_standard_user_gate_failed"
        message = Protect-SummaryText -Text $_.Exception.Message -SensitivePaths $sensitivePaths
    }
    $summary.checks = @([ordered]@{
        id = "windows_standard_user_release_suite"
        status = "failed"
        detailsHash = (Get-TextSha256 -Text $summary.failure.message)
    })
    Write-JsonUtf8NoBom -LiteralPath $summaryPath -Value $summary -Depth 8
    @(
        "status=failed",
        "administratorsMember=$($summary.administratorsMember)",
        "elevated=$($summary.elevated)",
        "previousVersion=$($summary.previousVersion)",
        "candidateVersion=$($summary.candidateVersion)",
        "previousInstallerSha256=$($summary.previousInstallerSha256)",
        "installerSha256=$($summary.installerSha256)",
        "desktopE2ESkipped=$($summary.desktopE2ESkipped)",
        "failure=$($summary.failure.message)"
    ) | Set-Content -LiteralPath (Join-Path $reportRoot "gate.sanitized.log") -Encoding UTF8
    throw
} finally {
    Remove-Item Env:\HACHIMI_DATA_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:\HACHIMI_STANDARD_USER_HACHIMI_EXE -ErrorAction SilentlyContinue
    Remove-Item Env:\HACHIMI_DESKTOP_E2E_REAL_SANDBOX -ErrorAction SilentlyContinue
    Remove-Item Env:\HACHIMI_SANDBOX_MARKER -ErrorAction SilentlyContinue
    Remove-Item Env:\HACHIMI_SANDBOX_LAUNCHER -ErrorAction SilentlyContinue
    Remove-Item Env:\HACHIMI_SANDBOX_CANARY -ErrorAction SilentlyContinue
    Remove-Item Env:\HACHIMI_SANDBOX_ATTESTATION_ROOT -ErrorAction SilentlyContinue
    Remove-Item Env:\HACHIMI_MANAGED_CHROMIUM -ErrorAction SilentlyContinue
    Remove-Item Env:\HACHIMI_RELEASE_WORKSPACE_WORKER -ErrorAction SilentlyContinue
    Pop-Location
}
