Add-Type @'
using System;
using System.Runtime.InteropServices;
public class WinUtil {
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
}
'@

$p = Get-Process CosmoDownloader -ErrorAction SilentlyContinue
if ($p) {
    $rect = New-Object WinUtil+RECT
    [WinUtil]::GetWindowRect($p.MainWindowHandle, [ref]$rect)
    Write-Host "Cosmo Window: Left=$($rect.Left), Top=$($rect.Top), Right=$($rect.Right), Bottom=$($rect.Bottom)"
} else {
    Write-Host "CosmoDownloader not running"
}
