param([string]$BuildDir = "$env:SystemDrive\vtd-build", [switch]$WithModel, [string]$OutputDir)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$output = if ($OutputDir) { [IO.Path]::GetFullPath($OutputDir) } else { Join-Path $root 'dist\vtd-windows' }
New-Item -ItemType Directory -Force $output | Out-Null
Copy-Item "$BuildDir\release\vtd.exe" $output
Copy-Item "$BuildDir\release\vtd-engine.exe" $output
# Do not generate or distribute this machine's preferences. The recipient's
# first run creates vtd.json using their Windows display language.
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
# Explicit contents keep old models and other leftover files out of future ZIPs.
$files = @('vtd.exe', 'vtd-engine.exe', 'LICENSE', 'README.md', 'THIRD_PARTY_LICENSES.txt',
    'download-model.ps1', 'Install.ps1', 'Uninstall.ps1', 'Autostart.ps1',
    'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
if ($WithModel) { $files += 'models/ggml-large-v3-turbo-q5_0.bin' }
$archive = Join-Path $root 'dist\vtd-windows-x64.zip'
$temporary = "$archive.tmp"
$sevenZip = (Get-Command 7z -ErrorAction SilentlyContinue).Source
if (-not $sevenZip -and (Test-Path "$env:ProgramFiles\7-Zip\7z.exe")) { $sevenZip = "$env:ProgramFiles\7-Zip\7z.exe" }
if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary }
if ($sevenZip) {
    Push-Location $output
    try {
        & $sevenZip a -tzip -mm=Deflate -mx=9 -mfb=258 -mpass=15 -mmt=1 -mtc=off -mta=off $temporary @files | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'ZIP compression failed' }
    } finally { Pop-Location }
} else {
    Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipArchive]::new([IO.File]::Create($temporary), [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($name in $files) {
            [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, (Join-Path $output $name), $name, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
        }
    } finally { $zip.Dispose() }
}
Move-Item -LiteralPath $temporary -Destination $archive -Force
Get-FileHash $archive -Algorithm SHA256
