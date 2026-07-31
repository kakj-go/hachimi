param(
    [string]$Version = "2.50.1.windows.1",
    [string]$ArchiveSha256 = "6f672aebe9e488a246efd6875f9197dbc0d9a40100e218acc3877cba2b206c45"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$utilityModule = Join-Path $PSHOME "Modules/Microsoft.PowerShell.Utility/Microsoft.PowerShell.Utility.psd1"
$archiveModule = Join-Path $PSHOME "Modules/Microsoft.PowerShell.Archive/Microsoft.PowerShell.Archive.psd1"
Import-Module -Name $utilityModule -ErrorAction Stop
Import-Module -Name $archiveModule -ErrorAction Stop

$workspaceRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$destination = [System.IO.Path]::GetFullPath((Join-Path $workspaceRoot "apps/desktop/src-tauri/managed-git"))
$expectedParent = [System.IO.Path]::GetFullPath((Join-Path $workspaceRoot "apps/desktop/src-tauri"))
if ([System.IO.Path]::GetDirectoryName($destination) -ne $expectedParent) {
    throw "Managed Git destination escaped the desktop package"
}

$archiveVersion = $Version -replace '\.windows\.\d+$', ''
$archiveName = "MinGit-$archiveVersion-64-bit.zip"
$sourceUrl = "https://github.com/git-for-windows/git/releases/download/v$Version/$archiveName"
$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("hachimi-managed-git-" + [guid]::NewGuid().ToString("N"))
$cacheRoot = Join-Path $workspaceRoot "target/hachimi-downloads"
$archivePath = Join-Path $cacheRoot $archiveName
$extractPath = Join-Path $temporaryRoot "expanded"

try {
    New-Item -ItemType Directory -Path $cacheRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $extractPath -Force | Out-Null
    if (Test-Path -LiteralPath $archivePath) {
        $cachedSha = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($cachedSha -ne $ArchiveSha256.ToLowerInvariant()) {
            Remove-Item -LiteralPath $archivePath -Force
        }
    }
    if (-not (Test-Path -LiteralPath $archivePath)) {
        $downloadPath = Join-Path $temporaryRoot ($archiveName + ".download")
        Invoke-WebRequest -Headers @{ "User-Agent" = "Hachimi-build" } -Uri $sourceUrl -OutFile $downloadPath
        Move-Item -LiteralPath $downloadPath -Destination $archivePath
    }
    $actualArchiveSha = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualArchiveSha -ne $ArchiveSha256.ToLowerInvariant()) {
        throw "Managed Git archive SHA-256 mismatch: expected $ArchiveSha256, got $actualArchiveSha"
    }
    Expand-Archive -LiteralPath $archivePath -DestinationPath $extractPath -Force
    if (-not (Test-Path -LiteralPath (Join-Path $extractPath "cmd/git.exe") -PathType Leaf)) {
        throw "Managed Git archive does not contain cmd/git.exe"
    }

    if (Test-Path -LiteralPath $destination) {
        $resolvedDestination = [System.IO.Path]::GetFullPath((Resolve-Path -LiteralPath $destination).Path)
        if ([System.IO.Path]::GetDirectoryName($resolvedDestination) -ne $expectedParent) {
            throw "Refusing to replace an unexpected Managed Git directory"
        }
        Remove-Item -LiteralPath $resolvedDestination -Recurse -Force
    }
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    Copy-Item -Path (Join-Path $extractPath "*") -Destination $destination -Recurse -Force

    $files = [ordered]@{}
    Get-ChildItem -LiteralPath $destination -File -Recurse | Sort-Object FullName | ForEach-Object {
        $relative = $_.FullName.Substring($destination.Length + 1).Replace('\', '/')
        if ($relative -eq "manifest.json") {
            throw "Managed Git archive unexpectedly contains Hachimi's manifest path"
        }
        $files[$relative] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $manifest = [ordered]@{
        version = $Version
        sourceUrl = $sourceUrl
        sourceArchiveSha256 = $ArchiveSha256.ToLowerInvariant()
        files = $files
    }
    $manifestPath = Join-Path $destination "manifest.json"
    $manifestJson = $manifest | ConvertTo-Json -Depth 5
    [System.IO.File]::WriteAllText($manifestPath, $manifestJson, [System.Text.UTF8Encoding]::new($false))
    Write-Host "Prepared pinned per-user Managed Git $Version with $($files.Count) attested files."
}
finally {
    if (Test-Path -LiteralPath $temporaryRoot) {
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
}
