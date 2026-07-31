param(
    [string]$Archive = "target/hachimi-downloads/chrome-win64-151.0.7922.47.zip",
    [string]$Destination = "apps/desktop/src-tauri/managed-chromium"
)

$ErrorActionPreference = "Stop"
$utilityModule = Join-Path $PSHOME "Modules/Microsoft.PowerShell.Utility/Microsoft.PowerShell.Utility.psd1"
$archiveModule = Join-Path $PSHOME "Modules/Microsoft.PowerShell.Archive/Microsoft.PowerShell.Archive.psd1"
Import-Module -Name $utilityModule -ErrorAction Stop
Import-Module -Name $archiveModule -ErrorAction Stop

$workspace = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$archivePath = [IO.Path]::GetFullPath((Join-Path $workspace $Archive))
$destinationPath = [IO.Path]::GetFullPath((Join-Path $workspace $Destination))
$expectedSha256 = "FC77BB98B550B7DA23B14EDFA282B59A022E7FDB075AC7625D2A5152CEB22396"

function Test-PreparedChromium {
    param([Parameter(Mandatory = $true)][string]$Root)

    $manifestPath = Join-Path $Root "manifest.json"
    if (-not (Test-Path -LiteralPath (Join-Path $Root "chrome.exe") -PathType Leaf) -or
        -not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        return $false
    }
    try {
        $entries = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        if ($null -eq $entries -or $entries.Count -eq 0) {
            return $false
        }
        $rootPrefix = $Root.TrimEnd([char[]]@('\', '/')) + [IO.Path]::DirectorySeparatorChar
        $hasChrome = $false
        foreach ($entry in $entries) {
            $relative = [string]$entry.path
            if ([string]::IsNullOrWhiteSpace($relative) -or [IO.Path]::IsPathRooted($relative)) {
                return $false
            }
            $candidate = [IO.Path]::GetFullPath((Join-Path $Root $relative))
            if (-not $candidate.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase) -or
                -not (Test-Path -LiteralPath $candidate -PathType Leaf) -or
                (Get-Item -LiteralPath $candidate).Length -ne [int64]$entry.size -or
                (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash -ne [string]$entry.sha256) {
                return $false
            }
            $normalized = $relative.Replace([char]'\', [char]'/')
            if ($normalized -eq "chrome.exe") {
                $hasChrome = $true
            }
        }
        return $hasChrome
    } catch {
        Write-Verbose "Prepared Managed Chromium validation failed: $($_.Exception.Message)"
        return $false
    }
}

if (Test-PreparedChromium -Root $destinationPath) {
    Write-Output "Prepared Managed Chromium already matches its per-file manifest."
    return
}

if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
    throw "Managed Chromium archive is missing: $archivePath. Download Chrome for Testing 151.0.7922.47 to the documented target path."
}
$actualSha256 = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash
if ($actualSha256 -ne $expectedSha256) {
    throw "Managed Chromium SHA-256 mismatch. Expected $expectedSha256, got $actualSha256."
}

$parent = Split-Path -Parent $destinationPath
$staging = Join-Path $parent (".managed-chromium-staging-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $staging | Out-Null
try {
    Expand-Archive -LiteralPath $archivePath -DestinationPath $staging -Force
    $root = Join-Path $staging "chrome-win64"
    if (-not (Test-Path -LiteralPath (Join-Path $root "chrome.exe") -PathType Leaf)) {
        throw "Chrome archive did not contain chrome-win64/chrome.exe."
    }
    if (Test-Path -LiteralPath $destinationPath) {
        Remove-Item -LiteralPath $destinationPath -Recurse -Force
    }
    Move-Item -LiteralPath $root -Destination $destinationPath
    $manifest = Get-ChildItem -LiteralPath $destinationPath -Recurse -File | ForEach-Object {
        $relative = $_.FullName.Substring($destinationPath.Length).TrimStart('\','/') -replace '\\','/'
        [PSCustomObject]@{
            path = $relative
            size = $_.Length
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    $manifestJson = $manifest | ConvertTo-Json -Depth 4
    [IO.File]::WriteAllText(
        (Join-Path $destinationPath "manifest.json"),
        $manifestJson,
        [Text.UTF8Encoding]::new($false)
    )
} finally {
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
}
