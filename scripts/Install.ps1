$ErrorActionPreference = 'Stop'
$exe = Join-Path $PSScriptRoot 'vtd.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw 'Extract the complete VTD package first. vtd.exe must be beside this script.'
}
$init = Start-Process -FilePath $exe -ArgumentList 'init' -WindowStyle Hidden -Wait -PassThru
if ($init.ExitCode -ne 0) { throw 'Cannot initialize VTD. Use a writable folder.' }
$config = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'vtd.json') -Raw | ConvertFrom-Json
$model = 'models/ggml-large-v3-turbo-q5_0.bin'
if ($config.model) { $model = $config.model }
if (-not [IO.Path]::IsPathRooted($model)) { $model = Join-Path $PSScriptRoot $model }
if (-not (Test-Path -LiteralPath $model -PathType Leaf)) {
    if ([IO.Path]::GetFileName($model) -ne 'ggml-large-v3-turbo-q5_0.bin') {
        throw "Configured model is missing: $model. Automatic setup downloads only the default Q5 model."
    }
    Write-Host 'Downloading the Q5 speech model (about 574 MB). Internet is needed for this first setup only.'
    & (Join-Path $PSScriptRoot 'download-model.ps1') -Quality q5_0 -Destination (Split-Path $model -Parent)
    if (-not (Test-Path -LiteralPath $model -PathType Leaf)) { throw 'Model download did not produce the expected file.' }
}

# Check first so running this script again does not start a second instance.
$status = Start-Process -FilePath $exe -ArgumentList 'status' -WindowStyle Hidden -Wait -PassThru
if ($status.ExitCode -ne 0) {
    $app = Start-Process -FilePath $exe -ArgumentList 'run' -WorkingDirectory $PSScriptRoot -WindowStyle Hidden -PassThru
    if ($app.WaitForExit(1500)) { throw 'VTD exited during startup. Autostart has not been changed.' }
}
& (Join-Path $PSScriptRoot 'Autostart.ps1') -Mode On
Write-Host 'VTD is running in the system tray. Hold F8 or toggle F9 to dictate.'
Write-Host 'Setup uses this folder directly; it does not copy files or require administrator rights.'
