$views = @('complete', 'failed', 'cancel')
foreach ($v in $views) {
    $outFile = "target/screenshots/view_$v.png"
    Write-Host "Capturing $outFile..."
    Start-Process -FilePath "target/debug/CosmoDownloader.exe" -ArgumentList @("--view=$v", "--screenshot=$outFile") -Wait
    Start-Sleep -Milliseconds 500
}
$scales = @('1.0', '1.25', '1.5')
foreach ($s in $scales) {
    $sName = $s.Replace('.', '')
    $outFile = "target/screenshots/view_scale_$sName.png"
    Write-Host "Capturing $outFile..."
    Start-Process -FilePath "target/debug/CosmoDownloader.exe" -ArgumentList @("--view=downloading", "--scale=$s", "--screenshot=$outFile") -Wait
    Start-Sleep -Milliseconds 500
}
Write-Host "All screenshots captured successfully!"
