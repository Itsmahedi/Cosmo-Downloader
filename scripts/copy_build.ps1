# PowerShell script to copy newest build of Cosmo Downloader to Windows Desktop
$ErrorActionPreference = "Stop"

$desktopPath = [Environment]::GetFolderPath('Desktop')
if (-not (Test-Path $desktopPath)) {
    $desktopPath = "$env:USERPROFILE\OneDrive\Desktop"
}
if (-not (Test-Path $desktopPath)) {
    $desktopPath = "$env:USERPROFILE\Desktop"
}

$projectRoot = Split-Path -Parent $PSScriptRoot
$targetDir = Join-Path $projectRoot "target"
$distDir = Join-Path $projectRoot "dist"

Write-Host "=== Copy Newest Cosmo Downloader Build to Desktop ===" -ForegroundColor Cyan
Write-Host "Desktop Path: $desktopPath" -ForegroundColor Gray

$candidates = @()

if (Test-Path $distDir) {
    $candidates += Get-ChildItem -Path $distDir -Filter "*.AppImage" -File -ErrorAction SilentlyContinue
}

foreach ($profile in @("release", "debug")) {
    $pdir = Join-Path $targetDir $profile
    if (Test-Path $pdir) {
        foreach ($bin in @("CosmoDownloader.exe", "CosmoDownloader", "cosmodownloader-cli.exe", "cosmodownloader-host.exe")) {
            $binPath = Join-Path $pdir $bin
            if (Test-Path $binPath) {
                $candidates += Get-Item $binPath
            }
        }
    }
}

# Also check inside Arch WSL target
$wslDistDir = "\\wsl.localhost\archlinux\home\steam\cosmo_downloader\dist"
$wslTargetDir = "\\wsl.localhost\archlinux\home\steam\cosmo_downloader\target"

if (Test-Path $wslDistDir) {
    $candidates += Get-ChildItem -Path $wslDistDir -Filter "*.AppImage" -File -ErrorAction SilentlyContinue
}
foreach ($profile in @("release", "debug")) {
    $wpdir = Join-Path $wslTargetDir $profile
    if (Test-Path $wpdir) {
        foreach ($bin in @("CosmoDownloader", "cosmodownloader-cli", "cosmodownloader-host")) {
            $binPath = Join-Path $wpdir $bin
            if (Test-Path $binPath) {
                $candidates += Get-Item $binPath
            }
        }
    }
}

if ($candidates.Count -eq 0) {
    Write-Warning "No built application binaries found in target or dist folders."
    Write-Host "Please build the project first (e.g. 'cargo build' or 'cargo build --release')."
    exit 1
}

$newest = $candidates | Sort-Object LastWriteTime -Descending | Select-Object -First 1

$dest = Join-Path $desktopPath $newest.Name
Copy-Item -Path $newest.FullName -Destination $dest -Force

$sizeMB = [math]::Round($newest.Length / 1MB, 2)
Write-Host "Successfully copied $($newest.Name) ($sizeMB MB) to Windows Desktop!" -ForegroundColor Green
Write-Host "Source: $($newest.FullName)" -ForegroundColor Gray
Write-Host "Destination: $dest" -ForegroundColor Gray
