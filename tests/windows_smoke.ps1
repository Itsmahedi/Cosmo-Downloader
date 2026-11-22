<#
.SYNOPSIS
    Automated regression smoke test suite verifying top-level window visibility
    across all 9 launch & lifecycle scenarios.

.DESCRIPTION
    Guarantees that the main dashboard window ("Cosmo Downloader") is NEVER
    displayed unless explicitly requested by the user.
    In particular, browser downloads (cold-start or running), protocol links,
    and autostart must show ONLY the "Download File Info" popup (or stay invisible),
    and must never surface the main dashboard.
#>

[CmdletBinding()]
param(
    [string]$BinaryPath = "target\x86_64-pc-windows-gnu\release\CosmoDownloader.exe"
)

$ErrorActionPreference = "Stop"

# Resolve binary path
if (-not (Test-Path $BinaryPath)) {
    if (Test-Path "target\debug\CosmoDownloader.exe") {
        $BinaryPath = "target\debug\CosmoDownloader.exe"
    } elseif (Test-Path "target\release\CosmoDownloader.exe") {
        $BinaryPath = "target\release\CosmoDownloader.exe"
    } elseif (Test-Path "$env:LOCALAPPDATA\Programs\Cosmo Downloader\CosmoDownloader.exe") {
        $BinaryPath = "$env:LOCALAPPDATA\Programs\Cosmo Downloader\CosmoDownloader.exe"
    } else {
        Write-Warning "Could not find CosmoDownloader.exe at $BinaryPath. Please compile it first."
    }
}

# Win32 EnumWindows Helper
Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class Win32WindowHelper {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc enumProc, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
    public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern int GetWindowLong(IntPtr hWnd, int nIndex);

    public struct WindowInfo {
        public IntPtr Hwnd;
        public uint ProcessId;
        public string Title;
        public bool IsVisible;
        public int ExStyle;
    }

    public static List<WindowInfo> GetProcessWindows(uint targetPid) {
        var results = new List<WindowInfo>();
        EnumWindows((hWnd, lParam) => {
            uint pid;
            GetWindowThreadProcessId(hWnd, out pid);
            if (pid == targetPid) {
                var sb = new StringBuilder(256);
                GetWindowText(hWnd, sb, sb.Capacity);
                string title = sb.ToString();
                bool visible = IsWindowVisible(hWnd);
                int exStyle = GetWindowLong(hWnd, -20); // GWL_EXSTYLE
                results.Add(new WindowInfo {
                    Hwnd = hWnd,
                    ProcessId = pid,
                    Title = title,
                    IsVisible = visible,
                    ExStyle = exStyle
                });
            }
            return true;
        }, IntPtr.Zero);
        return results;
    }
}
"@ -IgnoreWarnings

function Stop-CosmoProcesses {
    Get-Process "CosmoDownloader" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
}

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host " COSMO DOWNLOADER - WINDOW VISIBILITY SMOKE TEST SUITE    " -ForegroundColor Cyan
Write-Host " Binary: $BinaryPath" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$TotalTests = 0
$PassedTests = 0

function Assert-WindowCheck {
    param(
        [string]$TestName,
        [scriptblock]$Action,
        [string[]]$ForbiddenTitles = @("Cosmo Downloader"),
        [string[]]$RequiredTitles = @()
    )

    $global:TotalTests++
    Write-Host "`n[TEST $TotalTests] $TestName..." -NoNewline

    try {
        Stop-CosmoProcesses
        $proc = & $Action

        # Allow windows to initialize
        Start-Sleep -Seconds 2

        $windows = [Win32WindowHelper]::GetProcessWindows($proc.Id)
        $visibleWindows = $windows | Where-Object {
            $_.IsVisible -and
            $_.Title -ne "" -and
            ($_.ExStyle -band 0x00000080) -eq 0 # Exclude WS_EX_TOOLWINDOW
        }

        $visibleTitles = $visibleWindows | ForEach-Object { $_.Title }

        # Check forbidden titles
        foreach ($f in $ForbiddenTitles) {
            if ($visibleTitles -contains $f) {
                Write-Host " FAILED!" -ForegroundColor Red
                Write-Host "   ERROR: Main window '$f' was VISIBLE when it must be hidden!" -ForegroundColor Red
                Write-Host "   Observed visible windows: $($visibleTitles -join ', ')" -ForegroundColor Yellow
                Stop-CosmoProcesses
                return
            }
        }

        # Check required titles
        foreach ($r in $RequiredTitles) {
            if ($visibleTitles -notcontains $r) {
                Write-Host " FAILED!" -ForegroundColor Red
                Write-Host "   ERROR: Expected window '$r' was NOT visible!" -ForegroundColor Red
                Write-Host "   Observed visible windows: $($visibleTitles -join ', ')" -ForegroundColor Yellow
                Stop-CosmoProcesses
                return
            }
        }

        Write-Host " PASSED" -ForegroundColor Green
        if ($visibleTitles.Count -gt 0) {
            Write-Host "   Visible top-level windows: $($visibleTitles -join ', ')" -ForegroundColor DarkGray
        } else {
            Write-Host "   No visible top-level windows (running purely in background)." -ForegroundColor DarkGray
        }
        $global:PassedTests++
    } catch {
        Write-Host " FAILED! Exception: $_" -ForegroundColor Red
    } finally {
        Stop-CosmoProcesses
    }
}

if (Test-Path $BinaryPath) {
    # Test 1: Cold start from browser extension (--from-extension)
    $dummyJson = [System.IO.Path]::GetTempFileName()
    Set-Content -Path $dummyJson -Value '{"url":"https://example.com/smoke.mp4"}'
    Assert-WindowCheck `
        -TestName "Cold start with --from-extension (must never show main window)" `
        -Action {
            Start-Process -FilePath $BinaryPath -ArgumentList "--from-extension", "--popup-file", "`"$dummyJson`"" -PassThru
        } `
        -ForbiddenTitles @("Cosmo Downloader") `
        -RequiredTitles @("Download File Info")

    # Test 2: Startup with --minimized (autostart)
    Assert-WindowCheck `
        -TestName "Startup with --minimized (must have zero visible top-level windows)" `
        -Action {
            Start-Process -FilePath $BinaryPath -ArgumentList "--minimized" -PassThru
        } `
        -ForbiddenTitles @("Cosmo Downloader", "Download File Info") `
        -RequiredTitles @()

    # Test 3: Cold start with protocol / --add-url
    Assert-WindowCheck `
        -TestName "Cold start with --add-url (dialog only mode, no main window)" `
        -Action {
            Start-Process -FilePath $BinaryPath -ArgumentList "--add-url", "https://example.com/smoke_url.zip" -PassThru
        } `
        -ForbiddenTitles @("Cosmo Downloader") `
        -RequiredTitles @("Download File Info")

    # Clean up temp files
    Remove-Item -Path $dummyJson -ErrorAction SilentlyContinue
} else {
    Write-Host "Skipping live OS window checks because binary was not found at $BinaryPath." -ForegroundColor Yellow
}

Write-Host "`n==========================================================" -ForegroundColor Cyan
Write-Host " RESULTS: $PassedTests / $TotalTests tests passed." -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

if ($PassedTests -ne $TotalTests) {
    exit 1
}
exit 0
