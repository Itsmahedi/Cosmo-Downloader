# Automated Setup Script for Arch WSL & VS Code
$ErrorActionPreference = "Stop"

$distroName = "archlinux"
$wslImage = "$env:USERPROFILE\.wsl\archlinux.wsl"
$projectWinPath = "C:\Users\steam\OneDrive\Documents\cosmo_downloader"
$projectWslPath = "/mnt/c/Users/steam/OneDrive/Documents/cosmo_downloader"
$codeBin = "$env:LOCALAPPDATA\Programs\Microsoft VS Code\bin\code.cmd"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  Arch WSL & VS Code Environment Initializer" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

# 1. Check if Arch Linux is already installed in WSL
$installedDistros = wsl.exe -l -q 2>$null | ForEach-Object { $_.Trim().Replace("`0", "") }
$archInstalled = $installedDistros -contains $distroName

if (-not $archInstalled) {
    Write-Host "`n[1/3] Registering Arch Linux distribution..." -ForegroundColor Yellow
    if (Test-Path $wslImage) {
        Write-Host "Installing from local package: $wslImage" -ForegroundColor Gray
        wsl.exe --install --from-file $wslImage --name $distroName --no-launch
    } else {
        Write-Host "Downloading and installing via online catalog..." -ForegroundColor Gray
        wsl.exe --install -d $distroName --no-launch
    }
    Write-Host "Arch Linux installed successfully!" -ForegroundColor Green
} else {
    Write-Host "`n[1/3] Arch Linux is already installed in WSL." -ForegroundColor Green
}

# 2. Provision Arch Linux environment
Write-Host "`n[2/3] Provisioning development environment inside Arch Linux..." -ForegroundColor Yellow
Write-Host "(Installing Rust, Node, GitHub CLI, and Linux build dependencies...)" -ForegroundColor Gray

wsl.exe -d $distroName -u root -- bash -c "tr -d '\r' < '$projectWslPath/scripts/setup-arch-wsl.sh' | bash"

# 3. Launch VS Code in Arch WSL
Write-Host "`n[3/3] Launching VS Code connected to Arch WSL..." -ForegroundColor Yellow
if (Test-Path $codeBin) {
    & $codeBin --remote "wsl+$distroName" $projectWslPath
    Write-Host "VS Code launched successfully in Arch WSL!" -ForegroundColor Green
} else {
    Write-Host "VS Code CLI not found at default path. Launching WSL shell..." -ForegroundColor Yellow
    wsl.exe -d $distroName --cd $projectWslPath
}

Write-Host "`nSetup complete! Happy coding!" -ForegroundColor Cyan
