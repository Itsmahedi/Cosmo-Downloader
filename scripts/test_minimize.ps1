Add-Type @'
using System;
using System.Runtime.InteropServices;
public class WinUtil {
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
}
'@

$p = Get-Process CosmoDownloader -ErrorAction SilentlyContinue
if ($p) {
    Write-Host "Minimizing CosmoDownloader window..."
    [WinUtil]::ShowWindow($p.MainWindowHandle, 6) # SW_MINIMIZE
    Start-Sleep -Seconds 1
    
    $rect = New-Object WinUtil+RECT
    [WinUtil]::GetWindowRect($p.MainWindowHandle, [ref]$rect)
    Write-Host "Minimized Rect: Left=$($rect.Left), Top=$($rect.Top)"
}
