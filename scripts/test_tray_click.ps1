Add-Type @'
using System;
using System.Runtime.InteropServices;
public class WinUtil {
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")]
    public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, int dwExtraInfo);
    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int X, int Y);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
}
'@

$p = Get-Process CosmoDownloader -ErrorAction SilentlyContinue
if (!$p) {
    Write-Host "Process not found!"
    exit 1
}

Write-Host "1. Minimizing window..."
[WinUtil]::ShowWindow($p.MainWindowHandle, 6) # SW_MINIMIZE
Start-Sleep -Seconds 1

$rect = New-Object WinUtil+RECT
[WinUtil]::GetWindowRect($p.MainWindowHandle, [ref]$rect)
Write-Host "Minimized state: Left=$($rect.Left), Top=$($rect.Top)"

Write-Host "2. Clicking on Tray Icon at (1756, 1055)..."
[WinUtil]::SetCursorPos(1756, 1055)
Start-Sleep -Milliseconds 200
# MOUSEEVENTF_LEFTDOWN = 0x02, MOUSEEVENTF_LEFTUP = 0x04
[WinUtil]::mouse_event(0x02, 0, 0, 0, 0)
[WinUtil]::mouse_event(0x04, 0, 0, 0, 0)

Start-Sleep -Seconds 1

[WinUtil]::GetWindowRect($p.MainWindowHandle, [ref]$rect)
Write-Host "After Tray Click state: Left=$($rect.Left), Top=$($rect.Top), Right=$($rect.Right), Bottom=$($rect.Bottom)"
