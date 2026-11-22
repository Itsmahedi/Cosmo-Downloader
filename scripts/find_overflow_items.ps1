Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$root = [System.Windows.Automation.AutomationElement]::RootElement

$overflow_cond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ClassNameProperty,
    "NotifyIconOverflowWindow"
)
$overflow = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $overflow_cond)
if ($overflow) {
    Write-Host "Found NotifyIconOverflowWindow"
    $all = $overflow.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
    foreach ($el in $all) {
        $name = $el.Current.Name
        if ($name) {
            Write-Host "Overflow item: '$name' Rect=$($el.Current.BoundingRectangle)"
        }
    }
} else {
    Write-Host "NotifyIconOverflowWindow not open or not found"
}
