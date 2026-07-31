param(
    [int]$MinimumFreeGiB = 40
)

$ErrorActionPreference = "Stop"
$workspaceRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$volumeRoot = [System.IO.Path]::GetPathRoot($workspaceRoot)
if ([string]::IsNullOrWhiteSpace($volumeRoot)) {
    throw "release_disk_volume_unavailable"
}
$drive = [System.IO.DriveInfo]::new($volumeRoot)
$freeGiB = [Math]::Round($drive.AvailableFreeSpace / 1GB, 2)
if ($drive.AvailableFreeSpace -lt ($MinimumFreeGiB * 1GB)) {
    throw "release_disk_space_low: available=${freeGiB}GiB required=${MinimumFreeGiB}GiB"
}
Write-Output "release_disk_space_ok: available=${freeGiB}GiB required=${MinimumFreeGiB}GiB"
