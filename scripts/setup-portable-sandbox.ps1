$ErrorActionPreference = "Stop"

$scriptRoot = [System.IO.Path]::GetFullPath($PSScriptRoot)
$helper = Join-Path $scriptRoot "hachimi-sandbox-setup.exe"
$launcher = Join-Path $scriptRoot "hachimi-sandbox-launcher.exe"
$marker = Join-Path $scriptRoot "data\sandbox\windows\setup.json"

if (-not (Test-Path -LiteralPath $helper -PathType Leaf) -or -not (Test-Path -LiteralPath $launcher -PathType Leaf)) {
    throw "The portable Sandbox helper or launcher is missing."
}

& $helper --marker $marker --launcher $launcher
if ($LASTEXITCODE -ne 0) {
    throw "Sandbox setup helper failed with exit code $LASTEXITCODE."
}
$result = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
if (-not $result.networkComponent) {
    throw "Sandbox setup did not install the AppContainer deny-all network boundary."
}
Write-Host "Per-user AppContainer filesystem, process-tree, and deny-all network setup completed. Marker: $marker"
