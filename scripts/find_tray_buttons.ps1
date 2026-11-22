Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$root = [System.Windows.Automation.AutomationElement]::RootElement

$taskbar_cond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ClassNameProperty,
    "Shell_TrayWnd"
)
$taskbar = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $taskbar_cond)
if ($taskbar) {
    $btn_cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button
    )
    $buttons = $taskbar.FindAll([System.Windows.Automation.TreeScope]::Descendants, $btn_cond)
    foreach ($b in $buttons) {
        $n = $b.Current.Name
        $r = $b.Current.BoundingRectangle
        if ($r.Left -gt 1500) {
            Write-Host "Right side button: '$n' at $($r.Left),$($r.Top),$($r.Width),$($r.Height)"
        }
    }
}
