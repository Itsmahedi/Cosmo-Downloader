Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$root = [System.Windows.Automation.AutomationElement]::RootElement

# Find Taskbar
$taskbar_cond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ClassNameProperty,
    "Shell_TrayWnd"
)
$taskbar = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $taskbar_cond)
if ($taskbar) {
    Write-Host "Found Taskbar"
    $all = $taskbar.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
    foreach ($el in $all) {
        $name = $el.Current.Name
        $help = $el.Current.HelpText
        if ($name -or $help) {
            if ($name -match "Cosmo" -or $name -match "overflow" -or $name -match "chevron" -or $name -match "Notification" -or $help -match "Cosmo") {
                Write-Host "Element: Name='$name', Help='$help', Type=$($el.Current.ControlType.ProgrammaticName), Rect=$($el.Current.BoundingRectangle)"
            }
        }
    }
}
