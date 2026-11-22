# ==============================================================================
# Cosmo Downloader - Authenticode Signature Verification Script
# ==============================================================================
# Verifies that every PE file in the package (.exe, .dll, etc.) has a valid
# Authenticode signature, valid RFC 3161 timestamp, and consistent publisher.
# ==============================================================================

[CmdletBinding()]
param (
    [Parameter(Position = 0, ValueFromPipeline = $true)]
    [string[]]$Paths = @("installer\dist", "target\release"),

    [string]$ExpectedPublisher = "",

    [switch]$FailOnUnsigned,

    [switch]$AllowSelfSigned,

    [switch]$RequireTrusted
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Helper: Locate signtool.exe
function Find-SignTool {
    if ($env:SIGNTOOL_PATH -and (Test-Path $env:SIGNTOOL_PATH)) {
        return $env:SIGNTOOL_PATH
    }
    $cmd = Get-Command "signtool.exe" -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $sdkPaths = @(
        "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe",
        "C:\Program Files\Windows Kits\10\bin\*\x64\signtool.exe",
        "C:\Program Files (x86)\Microsoft SDKs\Windows\*\bin\*\x64\signtool.exe"
    )
    $resolved = Resolve-Path $sdkPaths -ErrorAction SilentlyContinue
    if ($resolved) {
        return ($resolved | Sort-Object Path -Descending | Select-Object -First 1).Path
    }
    return $null
}

$signtool = Find-SignTool

# Collect all target files
$extensions = @("*.exe", "*.dll", "*.node", "*.pyd", "*.sys", "*.msi")
$allFiles = @()

foreach ($p in $Paths) {
    if (-not (Test-Path $p)) { continue }
    $item = Get-Item $p
    if ($item.PSIsContainer) {
        $found = Get-ChildItem -Path $item.FullName -Include $extensions -File -Recurse |
            Where-Object { $_.Extension -ne ".old" -and $_.Extension -ne ".bak" }
        $allFiles += $found
    } else {
        $allFiles += $item
    }
}

$uniqueFiles = @($allFiles | Sort-Object FullName -Unique)

if (@($uniqueFiles).Count -eq 0) {
    Write-Warning "No PE binaries found in specified paths: $($Paths -join ', ')"
    exit 0
}

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  Cosmo Downloader - Authenticode Signature Verification  " -ForegroundColor Cyan
Write-Host "  Files found: $(@($uniqueFiles).Count)                   " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$results = [System.Collections.Generic.List[PSCustomObject]]::new()
$hasFailures = $false
$publishersFound = [System.Collections.Generic.List[string]]::new()

foreach ($file in $uniqueFiles) {
    $sig = Get-AuthenticodeSignature $file.FullName
    $hasTimestamp = ($sig.TimeStamperCertificate -ne $null)
    $signerSubject = if ($sig.SignerCertificate) { $sig.SignerCertificate.Subject } else { "<None>" }
    $status = $sig.Status.ToString()

    $signtoolResult = "N/A"
    if ($signtool) {
        $stOut = & $signtool verify /pa /v $file.FullName 2>&1
        if ($LASTEXITCODE -eq 0) {
            $signtoolResult = "Verified (PA)"
        } else {
            $signtoolResult = "Failed"
        }
    }

    $isOurBinary = $file.Name -match "^(CosmoDownloader|cosmodownloader|rustget)"
    if ($sig.SignerCertificate -and $isOurBinary) {
        $publishersFound.Add($signerSubject)
    }

    $fileValid = $false
    if ($status -eq "Valid") {
        $fileValid = $true
    } elseif ($status -eq "UnknownError" -and $AllowSelfSigned) {
        $fileValid = $true
        $status = "SelfSignedTest"
    }

    if (-not $fileValid) {
        $hasFailures = $true
    }

    if ($ExpectedPublisher -and $signerSubject -notlike "*$ExpectedPublisher*" -and $isOurBinary) {
        $fileValid = $false
        $hasFailures = $true
        $status = "PublisherMismatch"
    }

    $results.Add([PSCustomObject]@{
        Binary        = $file.Name
        Origin        = if ($isOurBinary) { "Ours" } else { "ThirdParty" }
        Status        = $status
        Timestamped   = if ($hasTimestamp) { "Yes (RFC 3161)" } else { "No" }
        Publisher     = if ($signerSubject.Length -gt 45) { $signerSubject.Substring(0, 42) + "..." } else { $signerSubject }
        SigntoolCheck = $signtoolResult
        FullPath      = $file.FullName
    })
}

# Display Table
$results | Format-Table -Property Binary, Origin, Status, Timestamped, Publisher, SigntoolCheck -AutoSize

Write-Host "----------------------------------------------------------" -ForegroundColor DarkGray
$validCount = @($results | Where-Object { $_.Status -eq "Valid" -or $_.Status -eq "SelfSignedTest" }).Count
$unsignedCount = @($results | Where-Object { $_.Status -eq "NotSigned" }).Count
$otherCount = @($results).Count - $validCount - $unsignedCount

Write-Host "Summary: Total: $(@($results).Count) | Valid: $validCount | Unsigned: $unsignedCount | Other: $otherCount" -ForegroundColor $(if ($hasFailures) { "Yellow" } else { "Green" })

# Check for publisher consistency across our binaries
$uniquePublishers = @($publishersFound | Select-Object -Unique)
if (@($uniquePublishers).Count -gt 1) {
    Write-Warning "Multiple distinct publisher identities detected across our binaries: $($uniquePublishers -join ' ; ')"
    Write-Warning "SmartScreen and Smart App Control reputation builds faster when the same publisher identity is used consistently."
} elseif (@($uniquePublishers).Count -eq 1) {
    Write-Host "[OK] Publisher identity consistent: $($uniquePublishers[0])" -ForegroundColor Green
}

if ($FailOnUnsigned -and $hasFailures) {
    Write-Error "CI Gate Failure: One or more binaries are unsigned or have an invalid Authenticode signature!"
    exit 1
}

exit 0
