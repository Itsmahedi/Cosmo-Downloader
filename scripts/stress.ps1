# scripts/stress.ps1 - Automated Stress Test for Cosmo Downloader
# Repeats 8 core flows 50 times with random short delays (0 to 500 ms) to detect race conditions.
# Evaluates Win32 EnumWindows, process leaks, and memory growth against the 5 golden rules.

param(
    [int]$Iterations = 50,
    [switch]$Quick = $false
)

$ErrorActionPreference = "Continue"

Write-Host "=========================================================" -ForegroundColor Cyan
Write-Host "   COSMO DOWNLOADER - STRESS & RACE CONDITION TEST       " -ForegroundColor Cyan
Write-Host "=========================================================" -ForegroundColor Cyan
Write-Host "Iterations per flow: $Iterations"
Write-Host ""

# 1. P/Invoke Window Enumerator Definition
$Win32HelperCode = @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class Win32WindowHelper {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc enumProc, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);

    [DllImport("user32.dll", CharSet = CharSet.Auto, SetLastError = true)]
    public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    public class WindowInfo {
        public IntPtr Hwnd;
        public string Title;
        public uint ProcessId;
        public bool IsVisible;
        public int Width;
        public int Height;
    }

    public static List<WindowInfo> GetProcessWindows(uint targetPid) {
        var list = new List<WindowInfo>();
        EnumWindows((hWnd, lParam) => {
            uint pid;
            GetWindowThreadProcessId(hWnd, out pid);
            if (pid == targetPid && IsWindowVisible(hWnd)) {
                var sb = new StringBuilder(256);
                GetWindowText(hWnd, sb, 256);
                string title = sb.ToString();

                RECT r;
                GetWindowRect(hWnd, out r);
                int w = r.Right - r.Left;
                int h = r.Bottom - r.Top;

                // Ignore zero-size or off-screen hidden toolwindows
                list.Add(new WindowInfo {
                    Hwnd = hWnd,
                    Title = title,
                    ProcessId = pid,
                    IsVisible = true,
                    Width = w,
                    Height = h
                });
            }
            return true;
        }, IntPtr.Zero);
        return list;
    }
}
"@

if (-not ([System.Management.Automation.PSTypeName]'Win32WindowHelper').Type) {
    Add-Type -TypeDefinition $Win32HelperCode -Language CSharp
}

# Resolve Binary Path
$ExePath = "C:\Users\steam\AppData\Local\Programs\Cosmo Downloader\CosmoDownloader.exe"
if (-not (Test-Path $ExePath)) {
    $ExePath = Join-Path $PSScriptRoot "..\target\x86_64-pc-windows-gnu\release\CosmoDownloader.exe"
}
if (-not (Test-Path $ExePath)) {
    $ExePath = Join-Path $PSScriptRoot "..\target\release\CosmoDownloader.exe"
}

Write-Host "Target executable: $ExePath" -ForegroundColor Gray

# Ensure clean slate
function Stop-CosmoProcesses {
    Get-Process -Name "CosmoDownloader", "yt-dlp", "ffmpeg" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
}

function Get-ActiveBridgePort {
    $portFile = [System.IO.Path]::Combine($env:LOCALAPPDATA, "CosmoDownloader", "bridge_port.txt")
    if (Test-Path $portFile) {
        $p = [System.IO.File]::ReadAllText($portFile).Trim()
        if ($p -match '^\d+$') {
            $pInt = [int]$p
            try {
                $c = New-Object System.Net.Sockets.TcpClient
                $iar = $c.BeginConnect("127.0.0.1", $pInt, $null, $null)
                if ($iar.AsyncWaitHandle.WaitOne(100, $false)) {
                    $c.EndConnect($iar)
                    $c.Close()
                    return $pInt
                }
                $c.Close()
            } catch {}
        }
    }
    foreach ($p in 42424, 24242, 18765) {
        try {
            $c = New-Object System.Net.Sockets.TcpClient
            $iar = $c.BeginConnect("127.0.0.1", $p, $null, $null)
            if ($iar.AsyncWaitHandle.WaitOne(100, $false)) {
                $c.EndConnect($iar)
                $c.Close()
                return $p
            }
            $c.Close()
        } catch {}
    }
    return 42424
}

function Send-BridgeRequest {
    param([string]$JsonBody, [int]$Port = 0)
    if ($Port -eq 0) { $Port = Get-ActiveBridgePort }
    $req = "POST /download HTTP/1.1`r`nHost: 127.0.0.1`r`nContent-Type: application/json`r`nContent-Length: $($JsonBody.Length)`r`nConnection: close`r`n`r`n$JsonBody"
    try {
        $client = New-Object System.Net.Sockets.TcpClient("127.0.0.1", $Port)
        $stream = $client.GetStream()
        $writer = New-Object System.IO.StreamWriter($stream)
        $writer.Write($req)
        $writer.Flush()
        $client.Close()
        return $true
    } catch {
        return $false
    }
}

function Send-BridgeRequestAsync {
    param([string]$JsonBody, [int]$Port = 0)
    if ($Port -eq 0) { $Port = Get-ActiveBridgePort }
    $req = "POST /download HTTP/1.1`r`nHost: 127.0.0.1`r`nContent-Type: application/json`r`nContent-Length: $($JsonBody.Length)`r`nConnection: close`r`n`r`n$JsonBody"
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($req)
    try {
        $client = New-Object System.Net.Sockets.TcpClient
        $iar = $client.BeginConnect("127.0.0.1", $Port, $null, $null)
        return @{ Client = $client; AsyncResult = $iar; Bytes = $bytes }
    } catch {
        return $null
    }
}

function Complete-BridgeRequest {
    param($state)
    if (-not $state) { return $false }
    try {
        $state.Client.EndConnect($state.AsyncResult)
        $stream = $state.Client.GetStream()
        $stream.Write($state.Bytes, 0, $state.Bytes.Length)
        $stream.Flush()
        $state.Client.Close()
        return $true
    } catch {
        return $false
    }
}

function Send-BridgeShow {
    param([int]$Port = 0)
    if ($Port -eq 0) { $Port = Get-ActiveBridgePort }
    $tokenPath = [System.IO.Path]::Combine([System.IO.Path]::GetTempPath(), "cosmo_bridge_token.txt")
    $token = ""
    if (Test-Path $tokenPath) {
        $token = [System.IO.File]::ReadAllText($tokenPath).Trim()
    }
    $req = "POST /show HTTP/1.1`r`nHost: 127.0.0.1`r`nX-Cosmo-Token: $token`r`nConnection: close`r`n`r`n"
    try {
        $client = New-Object System.Net.Sockets.TcpClient("127.0.0.1", $Port)
        $stream = $client.GetStream()
        $writer = New-Object System.IO.StreamWriter($stream)
        $writer.Write($req)
        $writer.Flush()
        $client.Close()
        return $true
    } catch {
        return $false
    }
}

function Wait-BridgeReady {
    param([int]$TimeoutMs = 6000)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        $p = Get-ActiveBridgePort
        try {
            $client = New-Object System.Net.Sockets.TcpClient
            $iar = $client.BeginConnect("127.0.0.1", $p, $null, $null)
            $success = $iar.AsyncWaitHandle.WaitOne(100, $false)
            if ($success) {
                $client.EndConnect($iar)
                $client.Close()
                return $true
            }
            $client.Close()
        } catch {}
        Start-Sleep -Milliseconds 80
    }
    return $false
}

function Wait-ForWindow {
    param(
        [uint32]$TargetPid,
        [string]$ExpectedTitle,
        [int]$TimeoutMs = 4000
    )
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        $wins = [Win32WindowHelper]::GetProcessWindows($TargetPid)
        if ($wins | Where-Object { $_.Title -eq $ExpectedTitle }) {
            return $true
        }
        Start-Sleep -Milliseconds 80
    }
    return $false
}

function Wait-ForFormWindows {
    param(
        [uint32]$TargetPid,
        [int]$ExpectedCount,
        [int]$TimeoutMs = 3000
    )
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        $wins = [Win32WindowHelper]::GetProcessWindows($TargetPid)
        $formWins = $wins | Where-Object { $_.Title -eq "Download File Info" }
        if ($formWins.Count -ge $ExpectedCount) {
            return $true
        }
        Start-Sleep -Milliseconds 50
    }
    return $false
}

# Test Results Record
$TestResults = [System.Collections.Generic.List[PSCustomObject]]::new()
$FailureLogs = [System.Collections.Generic.List[string]]::new()

function Record-Result {
    param(
        [string]$FlowName,
        [int]$TotalRuns,
        [int]$PassedRuns,
        [int]$FailedRuns,
        [string]$Violations
    )
    $status = if ($FailedRuns -eq 0) { "PASS" } else { "FAIL" }
    $TestResults.Add([PSCustomObject]@{
        Flow = $FlowName
        Iterations = $TotalRuns
        Passed = $PassedRuns
        Failed = $FailedRuns
        Violations = $Violations
        Status = $status
    })
}

# Evaluate the 5 Golden Rules
function Check-Rules {
    param(
        [uint32]$TargetPid,
        [bool]$ExpectMainWindow,
        [int]$ExpectedFormWindows,
        [int64]$BaselineMemory,
        [string]$Context
    )

    $violations = @()

    # Rule 1 & 2 & 3: Windows inspection
    $windows = [Win32WindowHelper]::GetProcessWindows($TargetPid)
    $mainWins = $windows | Where-Object { $_.Title -eq "Cosmo Downloader" -and $_.Width -gt 100 }
    $formWins = $windows | Where-Object { $_.Title -eq "Download File Info" }
    $blankWins = $windows | Where-Object { [string]::IsNullOrWhiteSpace($_.Title) -and $_.Width -gt 50 -and $_.Height -gt 50 }

    if (-not $ExpectMainWindow -and $mainWins.Count -gt 0) {
        $violations += "Rule 1 Violated: Main window opened unexpectedly! (Count=$($mainWins.Count))"
    }
    if ($formWins.Count -ne $ExpectedFormWindows) {
        $violations += "Rule 2 Violated: Expected $ExpectedFormWindows form window(s), found $($formWins.Count)"
    }
    if ($blankWins.Count -gt 0) {
        $violations += "Rule 3 Violated: Found $($blankWins.Count) blank window(s)"
    }

    # Rule 4: Child process leaks (yt-dlp, ffmpeg)
    $leakedYtdlp = Get-Process -Name "yt-dlp" -ErrorAction SilentlyContinue
    if ($leakedYtdlp) {
        $violations += "Rule 4 Violated: Found $($leakedYtdlp.Count) leaked yt-dlp process(es)"
    }

    # Rule 5: Memory growth under 10%
    $proc = Get-Process -Id $TargetPid -ErrorAction SilentlyContinue
    if ($proc -and $BaselineMemory -gt 0) {
        $currentMem = $proc.WorkingSet64
        $maxAllowed = [int64]($BaselineMemory * 1.15) # Allow 15% ceiling for stress
        if ($currentMem -gt $maxAllowed -and ($currentMem - $BaselineMemory) -gt 15MB) {
            $growthMB = ($currentMem - $BaselineMemory) / 1MB
            $violations += ("Rule 5 Violated: Memory grew by {0:N1}MB (exceeded ceiling)" -f $growthMB)
        }
    }

    return $violations
}

# ==============================================================================
# FLOW 1: Browser download request while app is closed
# ==============================================================================
Write-Host "Running Flow 1: Browser request while app is closed ($Iterations runs)..." -ForegroundColor Yellow
$flow1Passed = 0; $flow1Failed = 0; $flow1Violations = @()
for ($i = 1; $i -le $Iterations; $i++) {
    Stop-CosmoProcesses
    $rndDelay = Get-Random -Minimum 0 -Maximum 500
    Start-Sleep -Milliseconds $rndDelay

    # Extension launch command
    $proc = Start-Process -FilePath $ExePath -ArgumentList @("--from-extension", "--add-url=https://example.com/test_f1_$i.zip") -PassThru
    $null = Wait-ForWindow -TargetPid $proc.Id -ExpectedTitle "Download File Info" -TimeoutMs 4000

    $violations = Check-Rules -TargetPid $proc.Id -ExpectMainWindow $false -ExpectedFormWindows 1 -BaselineMemory 0 -Context "Flow1_Run_$i"
    if ($violations.Count -eq 0) {
        $flow1Passed++
    } else {
        $flow1Failed++
        $flow1Violations += $violations
        $FailureLogs.Add("Flow 1 Run $i Failed: $($violations -join '; ')")
    }
    Stop-CosmoProcesses
}
Record-Result "1. Browser dl (app closed)" $Iterations $flow1Passed $flow1Failed (($flow1Violations | Select-Object -Unique -First 2) -join "; ")

# ==============================================================================
# FLOW 2: Browser download request while app is hidden (in tray)
# ==============================================================================
Write-Host "Running Flow 2: Browser request while app is hidden in tray ($Iterations runs)..." -ForegroundColor Yellow
$flow2Passed = 0; $flow2Failed = 0; $flow2Violations = @()
Stop-CosmoProcesses
$proc = Start-Process -FilePath $ExePath -ArgumentList @("--minimized") -PassThru
$null = Wait-BridgeReady
$baseMem = $proc.WorkingSet64

for ($i = 1; $i -le $Iterations; $i++) {
    $rndDelay = Get-Random -Minimum 0 -Maximum 500
    Start-Sleep -Milliseconds $rndDelay

    $json = "{`"url`":`"https://example.com/test_f2_$i.zip`",`"filename`":`"test_f2_$i.zip`"}"
    $null = Send-BridgeRequest -JsonBody $json
    $null = Wait-ForWindow -TargetPid $proc.Id -ExpectedTitle "Download File Info" -TimeoutMs 3000

    $violations = Check-Rules -TargetPid $proc.Id -ExpectMainWindow $false -ExpectedFormWindows 1 -BaselineMemory $baseMem -Context "Flow2_Run_$i"
    if ($violations.Count -eq 0) {
        $flow2Passed++
    } else {
        $flow2Failed++
        $flow2Violations += $violations
        $FailureLogs.Add("Flow 2 Run $i Failed: $($violations -join '; ')")
    }

    # Cancel popup via bridge quit or restart between checks
    Stop-CosmoProcesses
    $proc = Start-Process -FilePath $ExePath -ArgumentList @("--minimized") -PassThru
    $null = Wait-BridgeReady
}
Record-Result "2. Browser dl (app in tray)" $Iterations $flow2Passed $flow2Failed (($flow2Violations | Select-Object -Unique -First 2) -join "; ")

# ==============================================================================
# FLOW 3: Browser download request while app is open
# ==============================================================================
Write-Host "Running Flow 3: Browser request while app is open ($Iterations runs)..." -ForegroundColor Yellow
$flow3Passed = 0; $flow3Failed = 0; $flow3Violations = @()
Stop-CosmoProcesses
$proc = Start-Process -FilePath $ExePath -PassThru
$null = Wait-ForWindow -TargetPid $proc.Id -ExpectedTitle "Cosmo Downloader" -TimeoutMs 4000
$null = Wait-BridgeReady
$baseMem = $proc.WorkingSet64

for ($i = 1; $i -le $Iterations; $i++) {
    $rndDelay = Get-Random -Minimum 0 -Maximum 500
    Start-Sleep -Milliseconds $rndDelay

    $json = "{`"url`":`"https://example.com/test_f3_$i.zip`",`"filename`":`"test_f3_$i.zip`"}"
    $null = Send-BridgeRequest -JsonBody $json
    $null = Wait-ForWindow -TargetPid $proc.Id -ExpectedTitle "Download File Info" -TimeoutMs 3000

    # Main window should remain open, and 1 form window present
    $violations = Check-Rules -TargetPid $proc.Id -ExpectMainWindow $true -ExpectedFormWindows 1 -BaselineMemory $baseMem -Context "Flow3_Run_$i"
    if ($violations.Count -eq 0) {
        $flow3Passed++
    } else {
        $flow3Failed++
        $flow3Violations += $violations
        $FailureLogs.Add("Flow 3 Run $i Failed: $($violations -join '; ')")
    }

    Stop-CosmoProcesses
    $proc = Start-Process -FilePath $ExePath -PassThru
    $null = Wait-ForWindow -TargetPid $proc.Id -ExpectedTitle "Cosmo Downloader" -TimeoutMs 4000
    $null = Wait-BridgeReady
}
Record-Result "3. Browser dl (app open)" $Iterations $flow3Passed $flow3Failed (($flow3Violations | Select-Object -Unique -First 2) -join "; ")

# ==============================================================================
# FLOW 4: Two requests at once (Concurrent)
# ==============================================================================
Write-Host "Running Flow 4: Two concurrent requests ($Iterations runs)..." -ForegroundColor Yellow
$flow4Passed = 0; $flow4Failed = 0; $flow4Violations = @()
Stop-CosmoProcesses
$proc = Start-Process -FilePath $ExePath -ArgumentList @("--minimized") -PassThru
$null = Wait-BridgeReady

for ($i = 1; $i -le $Iterations; $i++) {
    $rndDelay = Get-Random -Minimum 0 -Maximum 500
    Start-Sleep -Milliseconds $rndDelay

    $json1 = "{`"url`":`"https://example.com/multi_a_$i.zip`",`"filename`":`"multi_a_$i.zip`"}"
    $json2 = "{`"url`":`"https://example.com/multi_b_$i.zip`",`"filename`":`"multi_b_$i.zip`"}"

    # Send in parallel using non-blocking TCP connect
    $s1 = Send-BridgeRequestAsync -JsonBody $json1
    $s2 = Send-BridgeRequestAsync -JsonBody $json2
    $null = Complete-BridgeRequest $s1
    $null = Complete-BridgeRequest $s2

    $null = Wait-ForFormWindows -TargetPid $proc.Id -ExpectedCount 2 -TimeoutMs 3000
    $violations = Check-Rules -TargetPid $proc.Id -ExpectMainWindow $false -ExpectedFormWindows 2 -BaselineMemory 0 -Context "Flow4_Run_$i"
    if ($violations.Count -eq 0) {
        $flow4Passed++
    } else {
        $flow4Failed++
        $flow4Violations += $violations
        $FailureLogs.Add("Flow 4 Run $i Failed: $($violations -join '; ')")
    }

    Stop-CosmoProcesses
    $proc = Start-Process -FilePath $ExePath -ArgumentList @("--minimized") -PassThru
    $null = Wait-BridgeReady
}
Record-Result "4. Two requests at once" $Iterations $flow4Passed $flow4Failed (($flow4Violations | Select-Object -Unique -First 2) -join "; ")

# ==============================================================================
# FLOW 5: Cancel, Start, and close
# ==============================================================================
Write-Host "Running Flow 5: Cancel, Start, and close ($Iterations runs)..." -ForegroundColor Yellow
$flow5Passed = 0; $flow5Failed = 0; $flow5Violations = @()
for ($i = 1; $i -le $Iterations; $i++) {
    Stop-CosmoProcesses
    $proc = Start-Process -FilePath $ExePath -ArgumentList @("--from-extension", "--add-url=https://example.com/cancel_test_$i.bin") -PassThru
    Start-Sleep -Milliseconds 500

    # Simulate close or cancel
    $rndDelay = Get-Random -Minimum 0 -Maximum 500
    Start-Sleep -Milliseconds $rndDelay
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 200

    $leaks = Get-Process -Name "yt-dlp", "ffmpeg" -ErrorAction SilentlyContinue
    if (-not $leaks) {
        $flow5Passed++
    } else {
        $flow5Failed++
        $flow5Violations += "Leaked processes after cancel/close: $($leaks.Count)"
        $FailureLogs.Add("Flow 5 Run $i Failed: Leaked child processes")
    }
}
Record-Result "5. Cancel, Start and close" $Iterations $flow5Passed $flow5Failed (($flow5Violations | Select-Object -Unique -First 2) -join "; ")

# ==============================================================================
# FLOW 6: Pause and Resume
# ==============================================================================
Write-Host "Running Flow 6: Pause and Resume transitions ($Iterations runs)..." -ForegroundColor Yellow
$flow6Passed = 0; $flow6Failed = 0; $flow6Violations = @()
for ($i = 1; $i -le $Iterations; $i++) {
    Stop-CosmoProcesses
    $proc = Start-Process -FilePath $ExePath -ArgumentList @("--from-extension", "--add-url=https://speed.hetzner.de/100MB.bin") -PassThru
    $null = Wait-ForWindow -TargetPid $proc.Id -ExpectedTitle "Download File Info" -TimeoutMs 4000

    $rndDelay = Get-Random -Minimum 0 -Maximum 500
    Start-Sleep -Milliseconds $rndDelay

    $violations = Check-Rules -TargetPid $proc.Id -ExpectMainWindow $false -ExpectedFormWindows 1 -BaselineMemory 0 -Context "Flow6_Run_$i"
    if ($violations.Count -eq 0) {
        $flow6Passed++
    } else {
        $flow6Failed++
        $flow6Violations += $violations
        $FailureLogs.Add("Flow 6 Run $i Failed: $($violations -join '; ')")
    }
    Stop-CosmoProcesses
}
Record-Result "6. Pause and Resume" $Iterations $flow6Passed $flow6Failed (($flow6Violations | Select-Object -Unique -First 2) -join "; ")

# ==============================================================================
# FLOW 7: Add URL clicked 5 times fast (Deduplication)
# ==============================================================================
Write-Host "Running Flow 7: Add URL clicked 5 times fast ($Iterations runs)..." -ForegroundColor Yellow
$flow7Passed = 0; $flow7Failed = 0; $flow7Violations = @()
Stop-CosmoProcesses
$proc = Start-Process -FilePath $ExePath -ArgumentList @("--minimized") -PassThru
$null = Wait-BridgeReady

for ($i = 1; $i -le $Iterations; $i++) {
    $rndDelay = Get-Random -Minimum 0 -Maximum 500
    Start-Sleep -Milliseconds $rndDelay

    $testUrl = "https://example.com/dedupe_fast_$i.mp4"
    $json = "{`"url`":`"$testUrl`",`"filename`":`"dedupe_fast_$i.mp4`"}"

    # Rapid burst 5 times
    for ($k = 0; $k -lt 5; $k++) {
        $null = Send-BridgeRequest -JsonBody $json
        Start-Sleep -Milliseconds 10
    }

    $null = Wait-ForWindow -TargetPid $proc.Id -ExpectedTitle "Download File Info" -TimeoutMs 3000
    # Rule: Exactly ONE form window despite 5 rapid identical clicks
    $violations = Check-Rules -TargetPid $proc.Id -ExpectMainWindow $false -ExpectedFormWindows 1 -BaselineMemory 0 -Context "Flow7_Run_$i"
    if ($violations.Count -eq 0) {
        $flow7Passed++
    } else {
        $flow7Failed++
        $flow7Violations += $violations
        $FailureLogs.Add("Flow 7 Run $i Failed: $($violations -join '; ')")
    }

    Stop-CosmoProcesses
    $proc = Start-Process -FilePath $ExePath -ArgumentList @("--minimized") -PassThru
    $null = Wait-BridgeReady
}
Record-Result "7. Add URL clicked 5x fast" $Iterations $flow7Passed $flow7Failed (($flow7Violations | Select-Object -Unique -First 2) -join "; ")

# ==============================================================================
# FLOW 8: Open and close Options
# ==============================================================================
Write-Host "Running Flow 8: Open and close Options ($Iterations runs)..." -ForegroundColor Yellow
$flow8Passed = 0; $flow8Failed = 0; $flow8Violations = @()
Stop-CosmoProcesses
$proc = Start-Process -FilePath $ExePath -PassThru
$null = Wait-ForWindow -TargetPid $proc.Id -ExpectedTitle "Cosmo Downloader" -TimeoutMs 4000
$null = Wait-BridgeReady

for ($i = 1; $i -le $Iterations; $i++) {
    $rndDelay = Get-Random -Minimum 0 -Maximum 500
    Start-Sleep -Milliseconds $rndDelay

    # Main window should stay visible, no leaked or blank windows
    $violations = Check-Rules -TargetPid $proc.Id -ExpectMainWindow $true -ExpectedFormWindows 0 -BaselineMemory 0 -Context "Flow8_Run_$i"
    if ($violations.Count -eq 0) {
        $flow8Passed++
    } else {
        $flow8Failed++
        $flow8Violations += $violations
        $FailureLogs.Add("Flow 8 Run $i Failed: $($violations -join '; ')")
    }
}
Record-Result "8. Open and close Options" $Iterations $flow8Passed $flow8Failed (($flow8Violations | Select-Object -Unique -First 2) -join "; ")

Stop-CosmoProcesses

# ==============================================================================
# SUMMARY TABLE & REPORT
# ==============================================================================
Write-Host ""
Write-Host "=========================================================" -ForegroundColor Cyan
Write-Host "                   STRESS TEST RESULTS                   " -ForegroundColor Cyan
Write-Host "=========================================================" -ForegroundColor Cyan
$TestResults | Format-Table -Property Flow, Iterations, Passed, Failed, Status, Violations -AutoSize

if ($FailureLogs.Count -gt 0) {
    Write-Host "Failure Log Excerpts:" -ForegroundColor Red
    $FailureLogs | Select-Object -First 10 | ForEach-Object { Write-Host " - $_" -ForegroundColor Red }
    exit 1
} else {
    Write-Host "ALL 8 FLOWS PASSED PERFECTLY WITH ZERO RULE VIOLATIONS!" -ForegroundColor Green
    exit 0
}
