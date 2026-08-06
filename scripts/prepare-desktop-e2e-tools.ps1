param(
    [string]$Destination = "target/desktop-e2e-tools",
    [string]$TauriDriverVersion = "2.0.6"
)

$ErrorActionPreference = "Stop"

$utilityModule = Join-Path $PSHOME "Modules/Microsoft.PowerShell.Utility/Microsoft.PowerShell.Utility.psd1"
$archiveModule = Join-Path $PSHOME "Modules/Microsoft.PowerShell.Archive/Microsoft.PowerShell.Archive.psd1"
$securityModule = Join-Path $PSHOME "Modules/Microsoft.PowerShell.Security/Microsoft.PowerShell.Security.psd1"
Import-Module -Name $utilityModule -ErrorAction Stop
Import-Module -Name $archiveModule -ErrorAction Stop
Import-Module -Name $securityModule -ErrorAction Stop

$workspace = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$targetRoot = [IO.Path]::GetFullPath((Join-Path $workspace "target"))
$destinationPath = [IO.Path]::GetFullPath((Join-Path $workspace $Destination))
$targetPrefix = $targetRoot.TrimEnd([char[]]@('\', '/')) + [IO.Path]::DirectorySeparatorChar
if (-not $destinationPath.StartsWith($targetPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "desktop_e2e_tools_destination_invalid"
}

function Get-EdgeExecutable {
    $candidates = @(
        (Join-Path ${env:ProgramFiles(x86)} "Microsoft\Edge\Application\msedge.exe"),
        (Join-Path $env:ProgramFiles "Microsoft\Edge\Application\msedge.exe")
    )
    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            return $candidate
        }
    }
    throw "desktop_e2e_edge_missing"
}

function Assert-MicrosoftDriver {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$ExpectedVersion
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "desktop_e2e_msedgedriver_missing"
    }
    $actualVersion = (Get-Item -LiteralPath $Path).VersionInfo.FileVersion
    if ($actualVersion -ne $ExpectedVersion) {
        throw "desktop_e2e_msedgedriver_version_mismatch:$actualVersion"
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
        $signature.SignerCertificate.Subject -notmatch 'Microsoft Corporation') {
        throw "desktop_e2e_msedgedriver_signature_invalid"
    }
}

$edgeExecutable = Get-EdgeExecutable
$edgeVersion = (Get-Item -LiteralPath $edgeExecutable).VersionInfo.FileVersion
if ($edgeVersion -notmatch '^\d+\.\d+\.\d+\.\d+$') {
    throw "desktop_e2e_edge_version_invalid"
}
$tauriDriver = Join-Path $destinationPath "bin\tauri-driver.exe"
$edgeDriver = Join-Path $destinationPath "msedgedriver.exe"
$manifestPath = Join-Path $destinationPath "manifest.json"
$reuseTauriDriver = $false

if ((Test-Path -LiteralPath $manifestPath -PathType Leaf) -and
    (Test-Path -LiteralPath $tauriDriver -PathType Leaf) -and
    (Test-Path -LiteralPath $edgeDriver -PathType Leaf)) {
    try {
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        $reuseTauriDriver = $manifest.tauriDriverVersion -eq $TauriDriverVersion -and
            (Get-FileHash -LiteralPath $tauriDriver -Algorithm SHA256).Hash -eq $manifest.tauriDriverSha256
        if ($manifest.tauriDriverVersion -eq $TauriDriverVersion -and
            $manifest.edgeDriverVersion -eq $edgeVersion -and
            (Get-FileHash -LiteralPath $tauriDriver -Algorithm SHA256).Hash -eq $manifest.tauriDriverSha256 -and
            (Get-FileHash -LiteralPath $edgeDriver -Algorithm SHA256).Hash -eq $manifest.edgeDriverSha256) {
            Assert-MicrosoftDriver -Path $edgeDriver -ExpectedVersion $edgeVersion
            Write-Output "Prepared Desktop E2E tools already match their version and hash manifest."
            return
        }
    } catch {
        Write-Verbose "Existing Desktop E2E tool validation failed: $($_.Exception.Message)"
    }
}

$staging = Join-Path $targetRoot (".desktop-e2e-tools-staging-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $staging -Force | Out-Null
try {
    $stagedTauriRoot = Join-Path $staging "tauri"
    if ($reuseTauriDriver) {
        New-Item -ItemType Directory -Path (Join-Path $stagedTauriRoot "bin") -Force | Out-Null
        Copy-Item -LiteralPath $tauriDriver -Destination (Join-Path $stagedTauriRoot "bin\tauri-driver.exe")
    } else {
        $cargoTarget = Join-Path $staging "cargo-target"
        $previousCargoTarget = $env:CARGO_TARGET_DIR
        try {
            $env:CARGO_TARGET_DIR = $cargoTarget
            & cargo install tauri-driver --version $TauriDriverVersion --locked --root $stagedTauriRoot
            if ($LASTEXITCODE -ne 0) {
                throw "desktop_e2e_tauri_driver_install_failed"
            }
        } finally {
            $env:CARGO_TARGET_DIR = $previousCargoTarget
        }
    }
    $stagedTauriDriver = Join-Path $stagedTauriRoot "bin\tauri-driver.exe"
    $tauriHelp = (& $stagedTauriDriver --help 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0 -or $tauriHelp -notmatch 'USAGE:\s+tauri-driver') {
        throw "desktop_e2e_tauri_driver_invalid"
    }

    $edgeArchive = Join-Path $staging "edgedriver_win64.zip"
    $edgeUrl = "https://msedgedriver.microsoft.com/$edgeVersion/edgedriver_win64.zip"
    Invoke-WebRequest -Uri $edgeUrl -OutFile $edgeArchive -UseBasicParsing
    $edgeExtract = Join-Path $staging "edge"
    Expand-Archive -LiteralPath $edgeArchive -DestinationPath $edgeExtract -Force
    $stagedEdgeDriver = Join-Path $edgeExtract "msedgedriver.exe"
    Assert-MicrosoftDriver -Path $stagedEdgeDriver -ExpectedVersion $edgeVersion

    New-Item -ItemType Directory -Path (Join-Path $destinationPath "bin") -Force | Out-Null
    Copy-Item -LiteralPath $stagedTauriDriver -Destination $tauriDriver -Force
    Copy-Item -LiteralPath $stagedEdgeDriver -Destination $edgeDriver -Force
    $manifest = [ordered]@{
        schemaVersion = 1
        tauriDriverVersion = $TauriDriverVersion
        tauriDriverSha256 = (Get-FileHash -LiteralPath $tauriDriver -Algorithm SHA256).Hash.ToLowerInvariant()
        edgeDriverVersion = $edgeVersion
        edgeDriverSha256 = (Get-FileHash -LiteralPath $edgeDriver -Algorithm SHA256).Hash.ToLowerInvariant()
        edgeDriverCanonicalUrl = $edgeUrl
        acquiredAtUtc = [DateTime]::UtcNow.ToString("o")
    }
    [IO.File]::WriteAllText(
        $manifestPath,
        ($manifest | ConvertTo-Json -Depth 4),
        [Text.UTF8Encoding]::new($false)
    )
} finally {
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
}

Write-Output "Prepared tauri-driver $TauriDriverVersion and Microsoft Edge WebDriver $edgeVersion."
