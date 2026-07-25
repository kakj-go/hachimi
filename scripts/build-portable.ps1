$ErrorActionPreference = "Stop"

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$targetRoot = Join-Path $repoRoot "target"
$releaseRoot = Join-Path $targetRoot "release"
$portableRoot = Join-Path $targetRoot "portable"
$stageRoot = Join-Path $portableRoot "Hachimi"
$archivePath = Join-Path $portableRoot "Hachimi-portable.zip"

Push-Location $repoRoot
try {
    corepack pnpm tauri build --no-bundle --config apps/desktop/src-tauri/tauri.conf.json
    if ($LASTEXITCODE -ne 0) {
        throw "Tauri portable build failed with exit code $LASTEXITCODE"
    }

    if ([System.IO.Directory]::Exists($stageRoot)) {
        Get-ChildItem -LiteralPath $stageRoot -Recurse -Force -File -ErrorAction SilentlyContinue |
            ForEach-Object { [System.IO.File]::SetAttributes($_.FullName, [System.IO.FileAttributes]::Normal) }
        [System.IO.Directory]::Delete($stageRoot, $true)
    }
    if ([System.IO.File]::Exists($archivePath)) {
        [System.IO.File]::Delete($archivePath)
    }

    New-Item -ItemType Directory -Path $stageRoot -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $releaseRoot "hachimi-desktop.exe") -Destination $stageRoot
    Copy-Item -LiteralPath (Join-Path $releaseRoot "DirectML.dll") -Destination $stageRoot
    Copy-Item -LiteralPath (Join-Path $releaseRoot "onnxruntime.dll") -Destination $stageRoot
    Copy-Item -LiteralPath (Join-Path $releaseRoot "sherpa-onnx-c-api.dll") -Destination $stageRoot
    Copy-Item -LiteralPath (Join-Path $releaseRoot "resources") -Destination $stageRoot -Recurse
    Copy-Item -LiteralPath (Join-Path $repoRoot "scripts\reset-portable-data.ps1") -Destination $stageRoot
    Copy-Item -LiteralPath (Join-Path $repoRoot "scripts\reset-portable-data.cmd") -Destination $stageRoot
    New-Item -ItemType File -Path (Join-Path $stageRoot "hachimi.portable") -Force | Out-Null

    Compress-Archive -LiteralPath $stageRoot -DestinationPath $archivePath -CompressionLevel Optimal
    Write-Host "Portable package: $archivePath"
} finally {
    Pop-Location
}

