$WshShell = New-Object -ComObject WScript.Shell

$destTargets = @(
    "$env:USERPROFILE\OneDrive\Desktop\Cosmo Downloader (WSL Arch).lnk",
    "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Startup\Cosmo Downloader (WSL Arch).lnk"
)

$targetPath = "$env:LOCALAPPDATA\Programs\Microsoft VS Code\Code.exe"
$arguments = "--remote wsl+archlinux /home/steam/cosmo_downloader"
$iconPath = "$PSScriptRoot\..\icon.ico"

foreach ($dest in $destTargets) {
    $parent = Split-Path -Parent $dest
    if (-not (Test-Path $parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    $shortcut = $WshShell.CreateShortcut($dest)
    $shortcut.TargetPath = $targetPath
    $shortcut.Arguments = $arguments
    if (Test-Path $iconPath) {
        $shortcut.IconLocation = (Resolve-Path $iconPath).Path
    }
    $shortcut.Description = "Launch Cosmo Downloader in Arch WSL VS Code"
    $shortcut.Save()
    Write-Host "Created shortcut: $dest"
}
