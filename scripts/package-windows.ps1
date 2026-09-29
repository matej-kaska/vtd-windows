param([string]$BuildDir = "$env:SystemDrive\vtd-build", [switch]$WithModel)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$output = Join-Path $root 'dist\vtd-windows'
New-Item -ItemType Directory -Force $output | Out-Null
Copy-Item "$BuildDir\release\vtd.exe" $output
& "$output\vtd.exe" init
if ($LASTEXITCODE -ne 0) { throw 'Cannot initialize packaged executable' }
Copy-Item "$root\LICENSE", "$root\README.md" $output
foreach ($name in @('WINDOWS.md', 'BENCHMARKS.md')) {
    $obsolete = Join-Path $output $name
    if (Test-Path -LiteralPath $obsolete) { Remove-Item -LiteralPath $obsolete }
}
Copy-Item "$root\scripts\download-model.ps1" $output
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -property installationPath
$crt = Get-Item "$vs\VC\Redist\MSVC\*\x64\Microsoft.VC143.CRT" | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $crt) { throw 'MSVC redistributable runtime not found' }
Copy-Item "$($crt.FullName)\*.dll" $output
Copy-Item "$root\THIRD_PARTY_LICENSES.txt" $output
if ($WithModel) {
    New-Item -ItemType Directory -Force "$output\models" | Out-Null
    Copy-Item "$root\models\ggml-large-v3-turbo-q5_0.bin" "$output\models"
}
Compress-Archive -Path "$output\*" -DestinationPath "$root\dist\vtd-windows-x64.zip" -Force
Get-FileHash "$root\dist\vtd-windows-x64.zip" -Algorithm SHA256
