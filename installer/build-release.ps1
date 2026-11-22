# ==============================================================================
# Cosmo Downloader - Release Build, Authenticode Signing & Packaging Script
# ==============================================================================
param(
    [ValidateSet("Auto", "SignPath", "AzureTrustedSigning", "Signtool", "None", "SelfSignedTest")]
    [string]$Backend = "Auto",

    [switch]$SkipBuild = $false,

    [switch]$SkipInstaller = $false,

    [switch]$FailOnUnsigned = $false
)

if ($env:SIGNING_BACKEND -and $Backend -eq "Auto") {
    $Backend = $env:SIGNING_BACKEND
}

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RootDir = Split-Path -Parent $ScriptDir
$SignScript = Join-Path $RootDir "scripts\sign.ps1"
$VerifyScript = Join-Path $RootDir "scripts\verify_signatures.ps1"

Write-Host "======================================================" -ForegroundColor Cyan
Write-Host "     Cosmo Downloader Release & Packaging Pipeline    " -ForegroundColor Cyan
Write-Host "     Signing Backend: $Backend                        " -ForegroundColor Cyan
Write-Host "======================================================" -ForegroundColor Cyan

# 1. Setup Environment Paths
$llvmPkg = Get-Item "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\MartinStorsjo.LLVM-MinGW.UCRT_*\*\bin" -ErrorAction SilentlyContinue
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if ($llvmPkg) {
    $llvmBin = $llvmPkg.FullName
    $env:PATH = "$cargoBin;$llvmBin;$env:PATH"
    Write-Host "[OK] Toolchain LLVM-MinGW found at $llvmBin" -ForegroundColor Green
} else {
    $env:PATH = "$cargoBin;$env:PATH"
}

# 2. Generate Icons if needed
$iconPath = Join-Path $ScriptDir "icon.ico"
$extIconPath = Join-Path $RootDir "extension\icons\icon128.png"
if ((-not (Test-Path $iconPath)) -or (-not (Test-Path $extIconPath))) {
    Write-Host "[*] Generating application and extension icons..." -ForegroundColor Yellow
    $genScript = Join-Path $RootDir "scripts\generate_icons.ps1"
    & powershell -NoProfile -ExecutionPolicy Bypass -File $genScript
}

# 3. Cargo Release Build
if (-not $SkipBuild) {
    Write-Host "[*] Compiling Cosmo Downloader workspace in release mode..." -ForegroundColor Yellow
    Push-Location $RootDir
    try {
        # Check if x86_64-pc-windows-gnu is default or explicit
        cargo build --release --workspace
        if ($LASTEXITCODE -ne 0) {
            throw "Cargo release build failed with code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
    Write-Host "[OK] Release binaries compiled successfully!" -ForegroundColor Green
} else {
    Write-Host "[*] Skipping Cargo build (using precompiled release binaries)..." -ForegroundColor Yellow
}

# 4. Locate Binaries
$TargetReleaseDir = Join-Path $RootDir "target\x86_64-pc-windows-gnu\release"
if (-not (Test-Path (Join-Path $TargetReleaseDir "CosmoDownloader.exe"))) {
    $TargetReleaseDir = Join-Path $RootDir "target\release"
}

$cosmoGui  = Join-Path $TargetReleaseDir "CosmoDownloader.exe"
$cosmoCli  = Join-Path $TargetReleaseDir "cosmodownloader-cli.exe"
$cosmoHost = Join-Path $TargetReleaseDir "cosmodownloader-host.exe"
$legacyGui  = Join-Path $TargetReleaseDir "rustget-gui.exe"
$legacyCli  = Join-Path $TargetReleaseDir "rustget.exe"
$legacyHost = Join-Path $TargetReleaseDir "rustget-host.exe"

$sourceBinaries = @($cosmoGui, $cosmoCli, $cosmoHost, $legacyGui, $legacyCli, $legacyHost) | Where-Object { Test-Path $_ }
if ($sourceBinaries.Count -eq 0) {
    throw "No compiled binaries found in $TargetReleaseDir"
}

foreach ($bin in $sourceBinaries) {
    $sizeMb = [Math]::Round((Get-Item $bin).Length / 1MB, 2)
    $leaf = Split-Path -Leaf $bin
    Write-Host "  -> $leaf : $sizeMb MB" -ForegroundColor DarkGray
}

# Also check for yt-dlp.exe to sign before packaging
$ytdlpBin = Join-Path $RootDir "yt-dlp.exe"
$binariesToSign = @($sourceBinaries)
if (Test-Path $ytdlpBin) {
    $ytdlpSig = Get-AuthenticodeSignature $ytdlpBin
    if ($ytdlpSig.Status -ne "Valid") {
        Write-Host "[*] Including unsigned yt-dlp.exe in pre-packaging signing..." -ForegroundColor Yellow
        $binariesToSign += $ytdlpBin
    }
}

# ------------------------------------------------------------------------------
# STEP 2 (A): Sign our binaries and helper binaries FIRST before packaging
# ------------------------------------------------------------------------------
Write-Host "`n[*] [Signing Step 1/3] Signing application binaries before packaging..." -ForegroundColor Yellow
& $SignScript -Files $binariesToSign -Backend $Backend

# 5. Create Staging & Portable Package
$DistDir = Join-Path $RootDir "installer\dist"
if (Test-Path $DistDir) {
    Remove-Item $DistDir -Recurse -Force
}
New-Item -ItemType Directory -Path $DistDir | Out-Null

$PortableDir = Join-Path $DistDir "CosmoDownloader-v1.2.6-windows-x64"
New-Item -ItemType Directory -Path $PortableDir | Out-Null
$ExtDir = Join-Path $PortableDir "Browser-Extensions"
New-Item -ItemType Directory -Path $ExtDir | Out-Null

# Copy signed binaries to Portable staging
foreach ($bin in $sourceBinaries) {
    Copy-Item $bin -Destination "$PortableDir\" -Force
}
if (Test-Path $ytdlpBin) {
    Copy-Item $ytdlpBin -Destination "$PortableDir\" -Force
}
$ffmpegBin = Join-Path $RootDir "ffmpeg.exe"
if (Test-Path $ffmpegBin) {
    Copy-Item $ffmpegBin -Destination "$PortableDir\" -Force
}

Copy-Item (Join-Path $ScriptDir "icon.ico") -Destination "$PortableDir\" -Force
if (Test-Path (Join-Path $RootDir "LICENSE")) { Copy-Item (Join-Path $RootDir "LICENSE") -Destination "$PortableDir\" -Force }
if (Test-Path (Join-Path $RootDir "README.md")) { Copy-Item (Join-Path $RootDir "README.md") -Destination "$PortableDir\" -Force }

# Copy Browser Extensions
$extSrc = Join-Path $RootDir "browser-extensions"
if (Test-Path $extSrc) {
    Copy-Item "$extSrc\*" -Destination $ExtDir -Recurse -Force
}

# 6. Build Inno Setup Installer (Signs embedded unins000.exe via SignTool directive)
if (-not $SkipInstaller) {
    Write-Host "`n[*] [Packaging Step 2/3] Checking for Inno Setup compiler (ISCC.exe)..." -ForegroundColor Yellow
    $localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
    $isccPaths = @(
        (Join-Path $localAppData "Programs\Inno Setup 6\ISCC.exe"),
        "C:\Users\steam\AppData\Local\Programs\Antigravity IDE\resources\app\node_modules\innosetup\bin\ISCC.exe",
        "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
        "C:\Program Files\Inno Setup 6\ISCC.exe"
    )

    $iscc = $null
    foreach ($p in $isccPaths) {
        if (Test-Path $p) {
            $iscc = $p
            break
        }
    }

    if (-not $iscc) {
        $cmdCheck = Get-Command "ISCC.exe" -ErrorAction SilentlyContinue
        if ($cmdCheck) {
            $iscc = $cmdCheck.Source
        }
    }

    if ($iscc) {
        Write-Host "[*] Compiling installer with Inno Setup (ISCC)..." -ForegroundColor Yellow
        Push-Location $ScriptDir
        try {
            $isccArgs = @()
            if ($Backend -ne "None") {
                $signToolCmd = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$SignScript`" -Backend `"$Backend`" -Files `$f"
                $isccArgs += "/DSIGN_UNINSTALLER"
                $isccArgs += "/Ssigntool=$signToolCmd"
            }
            $isccArgs += "cosmodownloader.iss"

            if (Test-Path "cosmodownloader.iss") {
                Write-Host "  -> Compiling cosmodownloader.iss (Signing uninstaller: $(if ($Backend -ne 'None') { 'Yes' } else { 'No' }))..."
                & $iscc @isccArgs
                if ($LASTEXITCODE -eq 0) {
                    Write-Host "[OK] CosmoDownloader Windows Installer successfully built!" -ForegroundColor Green
                } else {
                    Write-Warning "Inno Setup compilation of cosmodownloader.iss exited with code $LASTEXITCODE"
                }
            }
        } finally {
            Pop-Location
        }

        # ----------------------------------------------------------------------
        # STEP 2 (B): Sign the compiled installer setup executable
        # ----------------------------------------------------------------------
        Write-Host "`n[*] [Signing Step 3/3] Signing compiled installer executable(s)..." -ForegroundColor Yellow
        $setupFiles = @(Get-ChildItem -Path $DistDir -Filter "*Setup*.exe" -File)
        if ($setupFiles.Count -gt 0) {
            & $SignScript -Files ($setupFiles | Select-Object -ExpandProperty FullName) -Backend $Backend
        }
    } else {
        Write-Host "------------------------------------------------------" -ForegroundColor Yellow
        Write-Host " Inno Setup (ISCC.exe) was not found on this system. " -ForegroundColor Yellow
        Write-Host " Portable build is fully ready in installer\dist\    " -ForegroundColor Green
        Write-Host "------------------------------------------------------" -ForegroundColor Yellow
    }
}

# 7. Compress Portable Zip (containing already-signed binaries)
$ZipPath = Join-Path $DistDir "CosmoDownloader-v1.2.6-windows-x64.zip"
Write-Host "`n[*] Creating portable ZIP archive: $ZipPath" -ForegroundColor Yellow
if (Test-Path $ZipPath) { Remove-Item $ZipPath -Force }
Compress-Archive -Path (Join-Path $PortableDir "*") -DestinationPath $ZipPath -Force
Write-Host "[OK] Portable package ready!" -ForegroundColor Green

# ------------------------------------------------------------------------------
# STEP 3: Automatic Verification of all PE binaries in the release package
# ------------------------------------------------------------------------------
Write-Host "`n[*] Running Authenticode Signature Verification Gate..." -ForegroundColor Yellow
$verifyParams = @{
    Paths = @($DistDir, $TargetReleaseDir)
}
if ($FailOnUnsigned) {
    $verifyParams["FailOnUnsigned"] = $true
}
if ($Backend -eq "SelfSignedTest") {
    $verifyParams["AllowSelfSigned"] = $true
}

& $VerifyScript @verifyParams

Write-Host ""
Write-Host "All release artifacts built, signed, and verified in: $DistDir" -ForegroundColor Cyan
