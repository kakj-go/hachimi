param(
    [Parameter(Mandatory = $true)]
    [string]$SourcePath
)

$ErrorActionPreference = "Stop"

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$assetDirectory = [IO.Path]::GetFullPath((Join-Path $repositoryRoot "assets/avatar-default/3800386813668044008"))
if (-not $assetDirectory.StartsWith($repositoryRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Resolved avatar asset path is outside the repository"
}

$expectedSha256 = "2954e74cf0258ca4f15360ac8aaa15ff7e3291d52a42fb5eb17709467963935a"
$destination = Join-Path $assetDirectory "3800386813668044008.vrm"

New-Item -ItemType Directory -Path $assetDirectory -Force | Out-Null
if (Test-Path -LiteralPath $destination) {
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $destination).Hash.ToLowerInvariant()
    if ($actual -ne $expectedSha256) {
        throw "Existing default avatar has an unexpected SHA-256: $actual"
    }
    Write-Output "Default avatar already prepared: $destination"
    exit 0
}

if (-not (Test-Path -LiteralPath $SourcePath -PathType Leaf)) {
    throw "Default avatar source is missing: $SourcePath"
}
$actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $SourcePath).Hash.ToLowerInvariant()
if ($actual -ne $expectedSha256) {
    throw "Default avatar source has an unexpected SHA-256: $actual"
}
Copy-Item -LiteralPath $SourcePath -Destination $destination
Write-Output "Prepared default avatar: $destination"
