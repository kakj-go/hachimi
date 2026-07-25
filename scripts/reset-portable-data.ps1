$ErrorActionPreference = "Stop"

$portableRoot = [System.IO.Path]::GetFullPath($PSScriptRoot).TrimEnd("\")
$dataRoot = Join-Path $portableRoot "data"

Get-Process -Name "hachimi-desktop" -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and $_.Path.StartsWith($portableRoot + "\", [System.StringComparison]::OrdinalIgnoreCase) } |
    Stop-Process -Force

if ([System.IO.Directory]::Exists($dataRoot)) {
    Get-ChildItem -LiteralPath $dataRoot -Recurse -Force -File -ErrorAction SilentlyContinue |
        ForEach-Object { [System.IO.File]::SetAttributes($_.FullName, [System.IO.FileAttributes]::Normal) }
    [System.IO.Directory]::Delete($dataRoot, $true)
}

$credentialTargets = @()
foreach ($line in (cmdkey.exe /list)) {
    if ($line -match '^\s*Target:\s+LegacyGeneric:target=(.+com\.hachimi\.desktop)\s*$') {
        $credentialTargets += $Matches[1].Trim()
    }
}
foreach ($targetName in ($credentialTargets | Sort-Object -Unique)) {
    cmdkey.exe "/delete:$targetName" | Out-Null
}

Write-Host "Hachimi portable data and credentials were removed."

