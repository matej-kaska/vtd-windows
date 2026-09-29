param([ValidateSet('On', 'Off')][string]$Mode = 'On')
$ErrorActionPreference = 'Stop'
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$name = 'VTD Windows'

if ($Mode -eq 'Off') {
    if (Test-Path -LiteralPath $key) {
        $entry = Get-ItemProperty -LiteralPath $key
        if ($entry.PSObject.Properties[$name]) {
            Remove-ItemProperty -LiteralPath $key -Name $name
        }
    }
    Write-Host 'VTD autostart is disabled. The running application is unchanged.'
    return
}

$exe = Join-Path $PSScriptRoot 'vtd.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw 'Extract the complete VTD package first. vtd.exe must be beside this script.'
}
$process = Start-Process -FilePath $exe -ArgumentList 'autostart', 'on' -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0) { throw 'Cannot enable VTD autostart.' }
Write-Host 'VTD will start when you sign in to Windows. Keep this folder in its current location.'
