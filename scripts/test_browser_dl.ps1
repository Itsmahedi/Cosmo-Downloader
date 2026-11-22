$body = @{
    url = 'https://www.youtube.com/watch?v=U_sQqg-0Uis'
    title = 'বাংলা নতুন গান ২০২৪ !!.mp4'
    filename = 'বাংলা নতুন গান ২০২৪ !!.mp4'
    file_size = 41943040
    category = 'Video'
} | ConvertTo-Json

Invoke-RestMethod -Uri 'http://127.0.0.1:42424/download' -Method Post -ContentType 'application/json; charset=utf-8' -Body $body

Start-Sleep -Milliseconds 600

Add-Type @'
using System;
using System.Runtime.InteropServices;
public class WinCheck {
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
}
'@

$proc = Get-Process -Name 'CosmoDownloader' | Select-Object -First 1
$rect = New-Object WinCheck+RECT
[WinCheck]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
$w = [int]($rect.Right - $rect.Left)
$h = [int]($rect.Bottom - $rect.Top)
Write-Output "PID: $($proc.Id) Title: '$($proc.MainWindowTitle)' Rect: Left=$([int]$rect.Left), Top=$([int]$rect.Top), W=$w, H=$h"
