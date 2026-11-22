# ==============================================================================
# Cosmo Downloader - Unified Code Signing Script (Authenticode)
# ==============================================================================
# Supports certificate-agnostic signing across:
#   (a) SignPath (GitHub Actions / Cloud CI integration)
#   (b) Azure Trusted Signing (Microsoft Artifact Signing)
#   (c) Hardware Token, Cloud HSM, or Local Cert with signtool.exe
#   (d) None (local unsigned dev builds)
#   (e) SelfSignedTest (local pipeline / syntax verification only)
# ==============================================================================

[CmdletBinding()]
param (
    [Parameter(Position = 0, Mandatory = $true, ValueFromPipeline = $true, ValueFromPipelineByPropertyName = $true)]
    [Alias("FullName", "Path", "f")]
    [string[]]$Files,

    [ValidateSet("Auto", "SignPath", "AzureTrustedSigning", "Signtool", "None", "SelfSignedTest")]
    [string]$Backend = "Auto",

    [string]$Description = "Cosmo Downloader",

    [string]$Url = "https://github.com/mahedi/cosmo-downloader",

    [string]$TimestampServer = "http://timestamp.digicert.com"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($env:SIGNING_BACKEND -and $Backend -eq "Auto") {
    $Backend = $env:SIGNING_BACKEND
}
if ($env:SIGNING_TIMESTAMP_URL -and $TimestampServer -eq "http://timestamp.digicert.com") {
    $TimestampServer = $env:SIGNING_TIMESTAMP_URL
}

$TimestampFallbacks = @(
    $TimestampServer,
    "http://timestamp.digicert.com",
    "http://timestamp.sectigo.com",
    "http://timestamp.globalsign.com/tsa/r6advanced1",
    "http://tsa.starfieldtech.com"
) | Select-Object -Unique

# ------------------------------------------------------------------------------
# Helper: Locate signtool.exe
# ------------------------------------------------------------------------------
function Find-SignTool {
    if ($env:SIGNTOOL_PATH -and (Test-Path $env:SIGNTOOL_PATH)) {
        return $env:SIGNTOOL_PATH
    }

    $cmd = Get-Command "signtool.exe" -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }

    $sdkPaths = @(
        "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe",
        "C:\Program Files\Windows Kits\10\bin\*\x64\signtool.exe",
        "C:\Program Files (x86)\Windows Kits\10\App Certification Kit\signtool.exe",
        "C:\Program Files\Windows Kits\10\App Certification Kit\signtool.exe",
        "C:\Program Files (x86)\Microsoft SDKs\Windows\*\bin\*\x64\signtool.exe"
    )

    $resolved = Resolve-Path $sdkPaths -ErrorAction SilentlyContinue
    if ($resolved) {
        return ($resolved | Sort-Object Path -Descending | Select-Object -First 1).Path
    }

    return $null
}

# ------------------------------------------------------------------------------
# Auto-detect Backend if set to "Auto"
# ------------------------------------------------------------------------------
if ($Backend -eq "Auto") {
    if ($env:SIGNPATH_API_TOKEN -or $env:SIGNPATH_ORGANIZATION_ID) {
        $Backend = "SignPath"
    } elseif ($env:AZURE_TRUSTED_SIGNING_ACCOUNT -or $env:AZURE_TRUSTED_SIGNING_ENDPOINT) {
        $Backend = "AzureTrustedSigning"
    } elseif ($env:SIGNING_CERT_THUMBPRINT -or $env:SIGNING_CERT_PATH -or $env:SIGNING_CSP -or $env:SIGNING_SUBJECT_NAME) {
        $Backend = "Signtool"
    } elseif ($env:SIGNING_TEST_CERT -or $env:USE_SELF_SIGNED_TEST_CERT) {
        $Backend = "SelfSignedTest"
    } else {
        $Backend = "None"
    }
}

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host " [Authenticode Signing] Backend: $Backend" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

# ------------------------------------------------------------------------------
# Expand and Validate Target Files
# ------------------------------------------------------------------------------
$TargetFiles = @()
foreach ($f in $Files) {
    if (-not $f) { continue }
    # Handle wildcard or direct paths
    $expanded = Resolve-Path $f -ErrorAction SilentlyContinue
    if ($expanded) {
        foreach ($item in $expanded) {
            $TargetFiles += $item.Path
        }
    } else {
        # Check if direct file exists
        if (Test-Path $f) {
            $TargetFiles += (Get-Item $f).FullName
        } else {
            Write-Warning "File not found for signing: $f"
        }
    }
}

if ($TargetFiles.Count -eq 0) {
    Write-Warning "No valid target files provided to sign.ps1"
    exit 0
}

# ------------------------------------------------------------------------------
# Execute Chosen Backend
# ------------------------------------------------------------------------------
switch ($Backend) {

    "None" {
        Write-Host "[*] Backend is 'None'. Skipping Authenticode signing (local unsigned dev build)." -ForegroundColor Yellow
        foreach ($file in $TargetFiles) {
            Write-Host "  -> Unsigned: $(Split-Path -Leaf $file)" -ForegroundColor DarkGray
        }
        exit 0
    }

    "SignPath" {
        Write-Host "[*] Executing SignPath signing..." -ForegroundColor Yellow
        $apiToken = $env:SIGNPATH_API_TOKEN
        $orgId = $env:SIGNPATH_ORGANIZATION_ID
        $projectSlug = $env:SIGNPATH_PROJECT_SLUG
        $policySlug = $env:SIGNPATH_SIGNING_POLICY_SLUG

        if (-not $apiToken -or -not $orgId -or -not $projectSlug -or -not $policySlug) {
            throw "SignPath backend requires environment variables: SIGNPATH_API_TOKEN, SIGNPATH_ORGANIZATION_ID, SIGNPATH_PROJECT_SLUG, SIGNPATH_SIGNING_POLICY_SLUG"
        }

        # Check for SignPath CLI if installed
        $signpathCli = Get-Command "signpath.exe" -ErrorAction SilentlyContinue
        foreach ($file in $TargetFiles) {
            Write-Host "  -> Submitting to SignPath: $(Split-Path -Leaf $file)" -ForegroundColor DarkGray
            if ($signpathCli) {
                & $signpathCli submit-signing-request `
                    --api-token $apiToken `
                    --organization-id $orgId `
                    --project-slug $projectSlug `
                    --signing-policy-slug $policySlug `
                    --input-file-path $file `
                    --output-file-path $file `
                    --wait-for-completion
                if ($LASTEXITCODE -ne 0) {
                    throw "SignPath signing failed for $file with exit code $LASTEXITCODE"
                }
            } else {
                # When running inside GitHub Actions with SignPath official integration,
                # signing is handled via the SignPath GitHub Action step.
                Write-Host "     SignPath CLI not found on local PATH. Ensure SignPath GitHub Action or CLI is active." -ForegroundColor Yellow
            }
        }
        Write-Host "[OK] SignPath signing finished." -ForegroundColor Green
    }

    "AzureTrustedSigning" {
        Write-Host "[*] Executing Azure Trusted Signing (Microsoft Artifact Signing)..." -ForegroundColor Yellow
        $endpoint = $env:AZURE_TRUSTED_SIGNING_ENDPOINT
        $account = $env:AZURE_TRUSTED_SIGNING_ACCOUNT
        $profile = $env:AZURE_TRUSTED_SIGNING_PROFILE

        if (-not $endpoint -or -not $account -or -not $profile) {
            throw "Azure Trusted Signing requires environment variables: AZURE_TRUSTED_SIGNING_ENDPOINT, AZURE_TRUSTED_SIGNING_ACCOUNT, AZURE_TRUSTED_SIGNING_PROFILE"
        }

        # Check for AzureSignTool or standard signtool with Azure dlib
        $azureSignTool = Get-Command "azuresigntool.exe" -ErrorAction SilentlyContinue
        $signtool = Find-SignTool

        foreach ($file in $TargetFiles) {
            Write-Host "  -> Signing with Azure Trusted Signing: $(Split-Path -Leaf $file)" -ForegroundColor DarkGray
            $signed = $false

            if ($azureSignTool) {
                # azuresigntool supports Azure CLI or Client Secret / Managed Identity
                $astArgs = @(
                    "sign",
                    "-kvu", $endpoint,
                    "-kvc", $profile,
                    "-kpn", $account,
                    "-fd", "sha256",
                    "-tr", $TimestampServer,
                    "-td", "sha256",
                    "-d", $Description,
                    "-du", $Url
                )
                if ($env:AZURE_CLIENT_ID -and $env:AZURE_CLIENT_SECRET -and $env:AZURE_TENANT_ID) {
                    $astArgs += @("-kvi", $env:AZURE_CLIENT_ID, "-kvs", $env:AZURE_CLIENT_SECRET, "-kvt", $env:AZURE_TENANT_ID)
                }

                $astArgs += $file
                & $azureSignTool @astArgs
                if ($LASTEXITCODE -eq 0) { $signed = $true }
            } elseif ($signtool -and $env:AZURE_TRUSTED_SIGNING_DLIB) {
                $dlib = $env:AZURE_TRUSTED_SIGNING_DLIB
                $metadata = $env:AZURE_TRUSTED_SIGNING_METADATA_JSON
                & $signtool sign /v /fd sha256 /tr $TimestampServer /td sha256 /dlib $dlib /dmdf $metadata $file
                if ($LASTEXITCODE -eq 0) { $signed = $true }
            } else {
                throw "Neither 'azuresigntool.exe' nor 'signtool.exe' with Azure Dlib found. Install AzureSignTool with: dotnet tool install --global AzureSignTool"
            }

            if (-not $signed) {
                throw "Azure Trusted Signing failed for file: $file"
            }
        }
        Write-Host "[OK] Azure Trusted Signing finished." -ForegroundColor Green
    }

    "Signtool" {
        $signtool = Find-SignTool
        if (-not $signtool) {
            Write-Warning "signtool.exe not found in Windows Kits or PATH. Checking for PowerShell fallback..."
        }

        foreach ($file in $TargetFiles) {
            Write-Host "  -> Signing: $(Split-Path -Leaf $file)" -ForegroundColor DarkGray
            $signed = $false

            # Try each timestamp fallback in sequence until timestamp succeeds
            foreach ($ts in $TimestampFallbacks) {
                if ($signtool) {
                    $signArgs = @("sign", "/v", "/fd", "sha256", "/tr", $ts, "/td", "sha256", "/d", $Description, "/du", $Url)

                    if ($env:SIGNING_CERT_PATH) {
                        $certPath = Resolve-Path $env:SIGNING_CERT_PATH -ErrorAction SilentlyContinue
                        if (-not $certPath) { throw "SIGNING_CERT_PATH specified but file does not exist: $env:SIGNING_CERT_PATH" }
                        $signArgs += @("/f", $certPath.Path)
                        if ($env:SIGNING_CERT_PASSWORD) {
                            $signArgs += @("/p", $env:SIGNING_CERT_PASSWORD)
                        }
                    } elseif ($env:SIGNING_CERT_THUMBPRINT) {
                        $signArgs += @("/sha1", $env:SIGNING_CERT_THUMBPRINT.Trim())
                        if ($env:SIGNING_CERT_STORE_MACHINE -eq "true") {
                            $signArgs += "/sm"
                        }
                    } elseif ($env:SIGNING_SUBJECT_NAME) {
                        $signArgs += @("/n", $env:SIGNING_SUBJECT_NAME)
                        if ($env:SIGNING_CERT_STORE_MACHINE -eq "true") {
                            $signArgs += "/sm"
                        }
                    } elseif ($env:SIGNING_CSP -and $env:SIGNING_KEY_CONTAINER) {
                        $signArgs += @("/csp", $env:SIGNING_CSP, "/kc", $env:SIGNING_KEY_CONTAINER)
                    } else {
                        # Default to auto-selecting best certificate in CurrentUser\My store
                        $signArgs += "/a"
                    }

                    $signArgs += $file
                    $out = & $signtool @signArgs 2>&1
                    if ($LASTEXITCODE -eq 0) {
                        $signed = $true
                        Write-Host "     Successfully signed with timestamp $ts" -ForegroundColor DarkGreen
                        break
                    } else {
                        Write-Warning "SignTool failed with timestamp $ts ($out). Trying next timestamp fallback..."
                    }
                } else {
                    # Fallback to Set-AuthenticodeSignature if signtool is missing but cert is provided
                    try {
                        $cert = $null
                        if ($env:SIGNING_CERT_PATH) {
                            $secPass = if ($env:SIGNING_CERT_PASSWORD) { ConvertTo-SecureString $env:SIGNING_CERT_PASSWORD -AsPlainText -Force } else { $null }
                            $cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2($env:SIGNING_CERT_PATH, $secPass)
                        } elseif ($env:SIGNING_CERT_THUMBPRINT) {
                            $cert = Get-Item "Cert:\CurrentUser\My\$($env:SIGNING_CERT_THUMBPRINT)" -ErrorAction SilentlyContinue
                            if (-not $cert) {
                                $cert = Get-Item "Cert:\LocalMachine\My\$($env:SIGNING_CERT_THUMBPRINT)" -ErrorAction SilentlyContinue
                            }
                        }

                        if ($cert) {
                            $res = Set-AuthenticodeSignature -FilePath $file -Certificate $cert -TimestampServer $ts -HashAlgorithm SHA256
                            if ($res.Status -eq "Valid") {
                                $signed = $true
                                Write-Host "     Successfully signed via Set-AuthenticodeSignature with timestamp $ts" -ForegroundColor DarkGreen
                                break
                            }
                        }
                    } catch {
                        Write-Warning "Set-AuthenticodeSignature failed with timestamp ${ts}: $($_.Exception.Message)"
                    }
                }
            }

            if (-not $signed) {
                throw "Authenticode signing failed for: $file across all timestamp servers and options."
            }
        }
        Write-Host "[OK] Signtool signing finished." -ForegroundColor Green
    }

    "SelfSignedTest" {
        Write-Host "==========================================================" -ForegroundColor Yellow
        Write-Host " [WARNING] RUNNING IN TEST SELF-SIGNED MODE                " -ForegroundColor Yellow
        Write-Host " Smart App Control (SAC) on Windows 11 enforces block     " -ForegroundColor Yellow
        Write-Host " Event 3077 on self-signed binaries! This mode is ONLY for" -ForegroundColor Yellow
        Write-Host " testing local script execution and pipeline integrity.   " -ForegroundColor Yellow
        Write-Host " Production releases MUST use a CA in MS Trusted Root!    " -ForegroundColor Yellow
        Write-Host "==========================================================" -ForegroundColor Yellow

        $certSubject = "CN=Cosmo Downloader Test Self-Signed, O=Cosmo Downloader Dev, C=US"
        $cert = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert | Where-Object { $_.Subject -eq $certSubject } | Select-Object -First 1

        if (-not $cert) {
            Write-Host "[*] Generating temporary self-signed Code Signing Certificate..." -ForegroundColor Yellow
            $cert = New-SelfSignedCertificate `
                -Type CodeSigningCert `
                -Subject $certSubject `
                -KeySpec Signature `
                -KeyExportPolicy Exportable `
                -KeyLength 2048 `
                -HashAlgorithm SHA256 `
                -CertStoreLocation "Cert:\CurrentUser\My" `
                -NotAfter (Get-Date).AddYears(1)
        }

        $signtool = Find-SignTool

        foreach ($file in $TargetFiles) {
            Write-Host "  -> Self-signing: $(Split-Path -Leaf $file)" -ForegroundColor DarkGray
            $signed = $false

            foreach ($ts in $TimestampFallbacks) {
                if ($signtool) {
                    $out = & $signtool sign /v /fd sha256 /tr $ts /td sha256 /sha1 $cert.Thumbprint /d $Description /du $Url $file 2>&1
                    if ($LASTEXITCODE -eq 0) {
                        $signed = $true
                        Write-Host "     Signed with signtool (Thumbprint: $($cert.Thumbprint))" -ForegroundColor DarkGreen
                        break
                    }
                } else {
                    $res = Set-AuthenticodeSignature -FilePath $file -Certificate $cert -TimestampServer $ts -HashAlgorithm SHA256
                    if ($res.Status -in @("Valid", "UnknownError")) {
                        $signed = $true
                        Write-Host "     Signed with Set-AuthenticodeSignature (Thumbprint: $($cert.Thumbprint))" -ForegroundColor DarkGreen
                        break
                    }
                }
            }

            if (-not $signed) {
                # Fallback without timestamp if network is unreachable in test mode
                $res = Set-AuthenticodeSignature -FilePath $file -Certificate $cert -HashAlgorithm SHA256
                if ($res.Status -in @("Valid", "UnknownError")) {
                    $signed = $true
                    Write-Host "     Signed without timestamp (local test fallback)" -ForegroundColor Yellow
                }
            }

            if (-not $signed) {
                throw "Self-signed test signing failed for file: $file"
            }
        }
        Write-Host "[OK] Self-signed test signing finished." -ForegroundColor Green
    }
}
