param(
    [string]$CacheDirectory = "target/speech-model-cache",
    [string]$Destination = "apps/desktop/src-tauri/resources/ai-models"
)

$ErrorActionPreference = "Stop"
$workspaceRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))

function Resolve-WorkspacePath([string]$RelativePath) {
    $path = [System.IO.Path]::GetFullPath((Join-Path $workspaceRoot $RelativePath))
    $prefix = $workspaceRoot.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
    if (-not $path.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Path must stay inside the Hachimi workspace: $RelativePath"
    }
    return $path
}

function Get-VerifiedArchive(
    [string]$Name,
    [string]$Url,
    [string]$Sha256,
    [string]$CacheRoot
) {
    $archive = Join-Path $CacheRoot $Name
    if (Test-Path -LiteralPath $archive) {
        $actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -eq $Sha256) {
            return $archive
        }
        Remove-Item -LiteralPath $archive
    }
    Invoke-WebRequest -Uri $Url -OutFile $archive
    $actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Sha256) {
        Remove-Item -LiteralPath $archive
        throw "SHA-256 mismatch for $Name. Expected $Sha256, got $actual."
    }
    return $archive
}

$cacheRoot = Resolve-WorkspacePath $CacheDirectory
$destinationRoot = Resolve-WorkspacePath $Destination
$buildRoot = Resolve-WorkspacePath ("target/speech-model-build-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $cacheRoot, $destinationRoot, $buildRoot | Out-Null

$vitsArchive = Get-VerifiedArchive `
    "vits-melo-tts-zh_en.tar.bz2" `
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-melo-tts-zh_en.tar.bz2" `
    "e58351ed7149f290a54534538badd4077cdbe6fddc964b24d0bee870415d1514" `
    $cacheRoot

$senseVoiceArchive = Get-VerifiedArchive `
    "sensevoice-small-int8.tar.bz2" `
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2" `
    "7305f7905bfcf77fa0b39388a313f3da35c68d971661a65475b56fb2162c8e63" `
    $cacheRoot

try {
    & tar.exe -xf $vitsArchive -C $buildRoot
    if ($LASTEXITCODE -ne 0) { throw "Failed to extract the MeloTTS VITS model." }

    $vitsSource = Join-Path $buildRoot "vits-melo-tts-zh_en"
    $vitsDestination = Join-Path $destinationRoot "text-to-speech/vits-melo-zh-en/vits-melo-tts-zh_en"

    $vitsModelRoot = [System.IO.Path]::GetFullPath((Split-Path $vitsDestination -Parent))
    $destinationPrefix = $destinationRoot.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
    if (-not $vitsModelRoot.StartsWith($destinationPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to replace a model directory outside the destination root: $vitsModelRoot"
    }
    $legacyVitsRoot = [System.IO.Path]::GetFullPath((Join-Path $destinationRoot "text-to-speech/vits-chaowen-int8"))
    if (-not $legacyVitsRoot.StartsWith($destinationPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to remove a legacy model directory outside the destination root: $legacyVitsRoot"
    }
    if (Test-Path -LiteralPath $legacyVitsRoot) {
        Remove-Item -LiteralPath $legacyVitsRoot -Recurse -Force
    }
    # Only replace generated artifacts. Documentation and notices in the model
    # directory are repository-owned and must survive a repair/download cycle.
    if (Test-Path -LiteralPath $vitsDestination) {
        Remove-Item -LiteralPath $vitsDestination -Recurse -Force
    }
    $vitsManifest = Join-Path $vitsModelRoot "manifest.json"
    if (Test-Path -LiteralPath $vitsManifest) {
        Remove-Item -LiteralPath $vitsManifest -Force
    }
    New-Item -ItemType Directory -Force -Path $vitsModelRoot | Out-Null

    New-Item -ItemType Directory -Force -Path $vitsDestination | Out-Null
    foreach ($name in @(
        "model.onnx", "tokens.txt", "lexicon.txt", "dict", "new_heteronym.fst",
        "phone.fst", "date.fst", "number.fst", "README.md", "LICENSE"
    )) {
        $source = Join-Path $vitsSource $name
        if (-not (Test-Path -LiteralPath $source)) {
            throw "MeloTTS archive is missing $name."
        }
        Copy-Item -LiteralPath $source -Destination $vitsDestination -Recurse
    }
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "model-manifests/vits-melo-zh-en.json") `
        -Destination $vitsManifest

    & tar.exe -xf $senseVoiceArchive -C $buildRoot
    if ($LASTEXITCODE -ne 0) { throw "Failed to extract the SenseVoice-Small model." }

    $senseVoiceSource = Join-Path $buildRoot "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09"
    $senseVoiceDestination = Join-Path $destinationRoot "speech-to-text/sensevoice-small"
    $resolvedSenseVoiceDestination = [System.IO.Path]::GetFullPath($senseVoiceDestination)
    if (-not $resolvedSenseVoiceDestination.StartsWith($destinationPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to replace a speech model directory outside the destination root: $resolvedSenseVoiceDestination"
    }
    # Keep tracked README/license files and replace only generated model data.
    New-Item -ItemType Directory -Force -Path $resolvedSenseVoiceDestination | Out-Null
    foreach ($name in @("model.int8.onnx", "tokens.txt", "manifest.json")) {
        $generatedPath = Join-Path $resolvedSenseVoiceDestination $name
        if (Test-Path -LiteralPath $generatedPath) {
            Remove-Item -LiteralPath $generatedPath -Force
        }
    }
    foreach ($name in @("model.int8.onnx", "tokens.txt")) {
        $source = Join-Path $senseVoiceSource $name
        if (-not (Test-Path -LiteralPath $source)) {
            throw "SenseVoice-Small archive is missing $name."
        }
        Copy-Item -LiteralPath $source -Destination $resolvedSenseVoiceDestination
    }
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "model-manifests/sensevoice-small.json") `
        -Destination (Join-Path $resolvedSenseVoiceDestination "manifest.json")

}
finally {
    if (Test-Path -LiteralPath $buildRoot) {
        Remove-Item -LiteralPath $buildRoot -Recurse -Force
    }
}

Write-Host "Prepared the MIT-licensed bilingual MeloTTS voice and bundled SenseVoice-Small under $destinationRoot."
