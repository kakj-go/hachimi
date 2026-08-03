param(
    [switch]$Release,
    [switch]$CheckOnly,
    [string]$BundleDirectory = "target/cef-bundle"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$toolingRoot = Join-Path $repoRoot "target/tooling"
$downloadRoot = Join-Path $toolingRoot "downloads"
$cmakeVersion = "3.31.8"
$ninjaVersion = "1.12.1"
$cmakeArchive = Join-Path $downloadRoot "cmake-$cmakeVersion-windows-x86_64.zip"
$ninjaArchive = Join-Path $downloadRoot "ninja-win-$ninjaVersion.zip"
$cmakeRoot = Join-Path $toolingRoot "cmake/cmake-$cmakeVersion-windows-x86_64"
$ninjaRoot = Join-Path $toolingRoot "ninja"
$cmakeExe = Join-Path $cmakeRoot "bin/cmake.exe"
$ninjaExe = Join-Path $ninjaRoot "ninja.exe"
$cefArchiveName = "cef_binary_151.3.14+g5d67476+chromium-151.0.7922.72_windows64_minimal.tar.bz2"
$cefArchiveSha256 = "C63A18909FEA077B5C3B5F9A3194F05781CD909EFA8A6D7A543CAD99C4183A55"

function Get-Sha256([string]$LiteralPath) {
    $stream = [System.IO.File]::OpenRead($LiteralPath)
    try {
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        try {
            return ([System.BitConverter]::ToString($sha256.ComputeHash($stream))).Replace("-", "")
        }
        finally {
            $sha256.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

$tools = @(
    @{
        Name = "CMake"
        Uri = "https://github.com/Kitware/CMake/releases/download/v$cmakeVersion/cmake-$cmakeVersion-windows-x86_64.zip"
        Archive = $cmakeArchive
        Sha256 = "81AA9964DBABD71FE02E7EC50472FD3AD56138C49944515ECE9001EFBFF8D719"
        Destination = Join-Path $toolingRoot "cmake"
        Executable = $cmakeExe
    },
    @{
        Name = "Ninja"
        Uri = "https://github.com/ninja-build/ninja/releases/download/v$ninjaVersion/ninja-win.zip"
        Archive = $ninjaArchive
        Sha256 = "F550FEC705B6D6FF58F2DB3C374C2277A37691678D6ABA463ADCBB129108467A"
        Destination = $ninjaRoot
        Executable = $ninjaExe
    }
)

New-Item -ItemType Directory -Force -Path $downloadRoot | Out-Null
foreach ($tool in $tools) {
    if (-not (Test-Path -LiteralPath $tool.Archive)) {
        Invoke-WebRequest -Uri $tool.Uri -OutFile $tool.Archive
    }
    $actualHash = Get-Sha256 $tool.Archive
    if ($actualHash -ne $tool.Sha256) {
        throw "$($tool.Name) archive checksum mismatch: expected $($tool.Sha256), got $actualHash"
    }
    if (-not (Test-Path -LiteralPath $tool.Executable)) {
        New-Item -ItemType Directory -Force -Path $tool.Destination | Out-Null
        Expand-Archive -LiteralPath $tool.Archive -DestinationPath $tool.Destination -Force
    }
}

$env:CMAKE = $cmakeExe
$env:PATH = "$ninjaRoot;$($env:PATH)"

Push-Location $repoRoot
try {
    if ($CheckOnly) {
        & cargo check -p hachimi-cef-host --lib
        if ($LASTEXITCODE -ne 0) { throw "CEF host check failed" }
        return
    }

    $profileArgument = if ($Release) { "--release" } else { $null }
    $buildArguments = @("build", "-p", "hachimi-cef-host", "--lib")
    if ($profileArgument) { $buildArguments += $profileArgument }
    & cargo @buildArguments
    if ($LASTEXITCODE -ne 0) { throw "CEF host build failed" }

    $cargoTargetRoot = if ($env:CARGO_TARGET_DIR) {
        if ([System.IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
            [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
        }
        else {
            [System.IO.Path]::GetFullPath((Join-Path $repoRoot $env:CARGO_TARGET_DIR))
        }
    }
    else {
        Join-Path $repoRoot "target"
    }
    $profileDirectoryName = if ($Release) { "release" } else { "debug" }
    $profileDirectory = Join-Path $cargoTargetRoot $profileDirectoryName
    $cefArchive = Get-ChildItem -LiteralPath (Join-Path $profileDirectory "build") -Filter $cefArchiveName -Recurse -File |
        Select-Object -First 1
    if (-not $cefArchive) {
        throw "CEF archive was not retained by the pinned cef build"
    }
    $actualCefHash = Get-Sha256 $cefArchive.FullName
    if ($actualCefHash -ne $cefArchiveSha256) {
        throw "CEF archive checksum mismatch: expected $cefArchiveSha256, got $actualCefHash"
    }
    $bundlePath = Join-Path $repoRoot $BundleDirectory
    $bundleArguments = @(
        "run", "-p", "hachimi-cef-host", "--bin", "bundle-hachimi-cef-host", "--",
        $bundlePath, $profileDirectory
    )
    if ($Release) { $bundleArguments = @("run", "--release", "-p", "hachimi-cef-host", "--bin", "bundle-hachimi-cef-host", "--", $bundlePath, $profileDirectory) }
    & cargo @bundleArguments
    if ($LASTEXITCODE -ne 0) { throw "CEF host bundling failed" }
} finally {
    Pop-Location
}
