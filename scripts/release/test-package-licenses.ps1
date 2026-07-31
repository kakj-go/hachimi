param(
    [Parameter(Mandatory = $true)][string]$PackageRoot
)

$ErrorActionPreference = "Stop"

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$packageRootPath = [System.IO.Path]::GetFullPath($PackageRoot)
if (-not (Test-Path -LiteralPath $packageRootPath -PathType Container)) {
    throw "release_package_root_missing"
}

function Assert-PackagedFile {
    param(
        [Parameter(Mandatory = $true)][string]$ExpectedPath,
        [Parameter(Mandatory = $true)][string]$StableId
    )
    $expected = [System.IO.Path]::GetFullPath($ExpectedPath)
    if (-not $expected.StartsWith($repoRoot, [StringComparison]::OrdinalIgnoreCase) -or
        -not (Test-Path -LiteralPath $expected -PathType Leaf)) {
        throw "release_package_source_invalid:$StableId"
    }
    $expectedHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $expected).Hash
    $name = [System.IO.Path]::GetFileName($expected)
    $matched = Get-ChildItem -LiteralPath $packageRootPath -Recurse -File -Filter $name |
        Where-Object { (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash -eq $expectedHash } |
        Select-Object -First 1
    if (-not $matched) {
        throw "release_package_notice_missing:$StableId"
    }
}

$required = [ordered]@{
    source_license = "LICENSE"
    root_notice = "NOTICE.md"
    default_avatar_notice = "assets\avatar-default\2639776812528692620\NOTICE.md"
    motion_clawatar_license = "assets\avatar-motions-v4\notices\clawatar-LICENSE.txt"
    motion_openmaiwaifu_license = "assets\avatar-motions-v4\notices\openmaiwaifu-LICENSE.txt"
    speech_third_party_notices = "apps\desktop\src-tauri\resources\ai-models\THIRD-PARTY-NOTICES.md"
    sensevoice_license = "apps\desktop\src-tauri\resources\ai-models\speech-to-text\sensevoice-small\LICENSE"
    melo_tts_license = "apps\desktop\src-tauri\resources\ai-models\text-to-speech\vits-melo-zh-en\vits-melo-tts-zh_en\LICENSE"
}

foreach ($entry in $required.GetEnumerator()) {
    Assert-PackagedFile -ExpectedPath (Join-Path $repoRoot $entry.Value) -StableId $entry.Key
}

$avatarManifestPath = Join-Path $repoRoot "assets\avatar-default\2639776812528692620\manifest.json"
$avatarManifest = Get-Content -LiteralPath $avatarManifestPath -Raw | ConvertFrom-Json
$avatar = Get-ChildItem -LiteralPath $packageRootPath -Recurse -File -Filter $avatarManifest.fileName |
    Where-Object {
        $_.Length -eq [int64]$avatarManifest.sizeBytes -and
        (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant() -eq
            ([string]$avatarManifest.sha256).ToLowerInvariant()
    } |
    Select-Object -First 1
if (-not $avatar) {
    throw "release_package_default_avatar_missing"
}

Write-Output "release_package_licenses_verified"
