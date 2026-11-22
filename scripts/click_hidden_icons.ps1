Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class WinUtil {
    [DllImport("user32.dll")]
    public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, int dwExtraInfo);
    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int X, int Y);
}
'@

# Click "Show Hidden Icons" at (1658, 1056)
Write-Host "Clicking 'Show Hidden Icons'..."
[WinUtil]::SetCursorPos(1658, 1056)
Start-Sleep -Milliseconds 100
[WinUtil]::mouse_event(0x02, 0, 0, 0, 0) # Down
[WinUtil]::mouse_event(0x04, 0, 0, 0, 0) # Up
Start-Sleep -Seconds 1

$root = [System.Windows.Automation.AutomationElement]::RootElement
$btn_cond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::Button
)
$buttons = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $btn_cond)
foreach ($b in $buttons) {
    $n = $b.Current.Name
    $r = $b.Current.BoundingRectangle
    if ($n -match "Cosmo" -or $n -match "Downloader") {
        Write-Host "FOUND COSMO IN TRAY/OVERFLOW: Name='$n' Rect=$($r.Left),$($r.Top),$($r.Width),$($r.Height)"
    }
}
