Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$root = [System.Windows.Automation.AutomationElement]::RootElement

# Find all elements under Notification Area or System Tray
$condition = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ClassNameProperty,
    "TopLevelWindowForOverflowXamlIsland"
)
$overflow = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)

$notify_cond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ClassNameProperty,
    "ToolbarWindow32"
)
$toolbars = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $notify_cond)
foreach ($tb in $toolbars) {
    Write-Host "Toolbar: $($tb.Current.Name) / $($tb.Current.AutomationId)"
    $child_buttons = $tb.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)
    foreach ($b in $child_buttons) {
        Write-Host "  Tray item: '$($b.Current.Name)' Rect=$($b.Current.BoundingRectangle)"
    }
}
