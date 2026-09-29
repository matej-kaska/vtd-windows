$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'Autostart.ps1') -Mode Off

$exe = Join-Path $PSScriptRoot 'vtd.exe'
$session = (Get-Process -Id $PID).SessionId
$running = @(Get-Process -Name vtd -ErrorAction SilentlyContinue | Where-Object {
    $_.SessionId -eq $session -and $_.Path -eq $exe
})
if ($running.Count -gt 0) {
    $stop = Start-Process -FilePath $exe -ArgumentList 'stop' -WindowStyle Hidden -Wait -PassThru
    if ($stop.ExitCode -ne 0) { throw 'Cannot stop VTD. Close it from the system tray, then run this script again.' }
    try {
        $running | Wait-Process -Timeout 15 -ErrorAction Stop
    } catch {
        throw 'VTD has not finished exiting. Wait for transcription to finish, then run this script again.'
    }
}
Write-Host 'VTD has been removed from autostart and is no longer running from this folder.'
Write-Host 'You can now delete this folder. The model and configuration have been kept.'
