param([string]$BuildDir = "$env:SystemDrive\vtd-build", [switch]$WithModel, [string]$OutputDir)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$output = if ($OutputDir) { [IO.Path]::GetFullPath($OutputDir) } else { Join-Path $root 'dist\vtd-windows' }
New-Item -ItemType Directory -Force $output | Out-Null
Copy-Item "$BuildDir\release\vtd.exe" $output
Copy-Item "$BuildDir\release\vtd-engine.exe" $output
$init = Start-Process -FilePath "$output\vtd.exe" -ArgumentList 'init' -WindowStyle Hidden -Wait -PassThru
if ($init.ExitCode -ne 0) { throw 'Cannot initialize packaged executable' }
Copy-Item "$root\LICENSE", "$root\README.md" $output
foreach ($name in @('WINDOWS.md', 'BENCHMARKS.md')) {
    $obsolete = Join-Path $output $name
    if (Test-Path -LiteralPath $obsolete) { Remove-Item -LiteralPath $obsolete }
}
Copy-Item "$root\scripts\download-model.ps1" $output
foreach ($name in @('Install.ps1', 'Uninstall.ps1', 'Autostart.ps1')) {
    Copy-Item -LiteralPath (Join-Path "$root\scripts" $name) -Destination $output
}
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -property installationPath
$crt = Get-Item "$vs\VC\Redist\MSVC\*\x64\Microsoft.VC143.CRT" | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $crt) { throw 'MSVC redistributable runtime not found' }
foreach ($name in @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')) {
    Copy-Item (Join-Path $crt.FullName $name) $output
}
foreach ($name in @('concrt140.dll', 'msvcp140_1.dll', 'msvcp140_2.dll', 'msvcp140_atomic_wait.dll', 'msvcp140_codecvt_ids.dll', 'vccorlib140.dll', 'vcruntime140_threads.dll')) {
    $obsolete = Join-Path $output $name
    if (Test-Path -LiteralPath $obsolete) { Remove-Item -LiteralPath $obsolete }
}
Copy-Item "$root\THIRD_PARTY_LICENSES.txt" $output
if ($WithModel) {
    New-Item -ItemType Directory -Force "$output\models" | Out-Null
    $source = Join-Path $root 'models\ggml-large-v3-turbo-q5_0.bin'
    $target = Join-Path $output 'models\ggml-large-v3-turbo-q5_0.bin'
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw 'Download the model first.' }
    if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target }
    try { New-Item -ItemType HardLink -Path $target -Target $source -ErrorAction Stop | Out-Null }
    catch { Copy-Item -LiteralPath $source -Destination $target }
}
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipArchive]::new([IO.File]::Create("$root\dist\vtd-windows-x64.zip"), [IO.Compression.ZipArchiveMode]::Create)
try {
    # Explicit contents keep old models and other leftover files out of future ZIPs.
    $files = @('vtd.exe', 'vtd-engine.exe', 'vtd.json', 'LICENSE', 'README.md', 'THIRD_PARTY_LICENSES.txt',
        'download-model.ps1', 'Install.ps1', 'Uninstall.ps1', 'Autostart.ps1',
        'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
    if ($WithModel) { $files += 'models/ggml-large-v3-turbo-q5_0.bin' }
    foreach ($name in $files) {
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, (Join-Path $output $name), $name, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
} finally { $zip.Dispose() }
Get-FileHash "$root\dist\vtd-windows-x64.zip" -Algorithm SHA256
