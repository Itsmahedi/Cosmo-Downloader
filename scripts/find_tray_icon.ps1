Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$root = [System.Windows.Automation.AutomationElement]::RootElement

# Find Notification Area or Taskbar
$condition = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::Button
)
$buttons = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
foreach ($btn in $buttons) {
    $name = $btn.Current.Name
    if ($name -match "Cosmo" -or $name -match "Downloader") {
        $rect = $btn.Current.BoundingRectangle
        Write-Host "Found Cosmo Tray Button: Name='$name', Rect=$rect"
        $clickX = [int]($rect.Left + $rect.Width / 2)
        $clickY = [int]($rect.Top + $rect.Height / 2)
        Write-Host "Click coordinate: ($clickX, $clickY)"
    }
}
