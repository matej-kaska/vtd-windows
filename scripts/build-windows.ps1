param([switch]$Test, [string]$BuildDir = "$env:SystemDrive\vtd-build")
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
if (Test-Path "$root\.tools\cargo\bin\cargo.exe") {
    $env:CARGO_HOME = "$root\.tools\cargo"
    $env:RUSTUP_HOME = "$root\.tools\rustup"
    $env:PATH = "$root\.tools\cargo\bin;$env:PATH"
}
if (Test-Path "$root\.tools\vulkan\Bin\glslc.exe") { $env:VULKAN_SDK = "$root\.tools\vulkan" }
if (-not $env:VULKAN_SDK) { throw 'Install Vulkan SDK and set VULKAN_SDK first.' }
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -property installationPath
$env:PATH = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer;$env:PATH"
$devcmd = '"' + $vs + '\Common7\Tools\VsDevCmd.bat" -arch=x64 -host_arch=x64 >nul && set'
cmd /c $devcmd | ForEach-Object { if ($_ -match '^([^=]+)=(.*)$') { [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process') } }
$env:PATH = "$vs\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin;$vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja;$env:PATH"
$env:CMAKE_GENERATOR = 'Ninja'
$env:CARGO_TARGET_DIR = $BuildDir
if (Test-Path "$root\.tools\libclang\clang\native\libclang.dll") { $env:LIBCLANG_PATH = "$root\.tools\libclang\clang\native" }
Remove-Item Env:\WHISPER_DONT_GENERATE_BINDINGS -ErrorAction SilentlyContinue
$env:CMAKE_BUILD_PARALLEL_LEVEL = '8'
$env:GGML_NATIVE = 'OFF'
if ($Test) { cargo test --release --locked } else { cargo build --release --locked }
if ($LASTEXITCODE -ne 0) { throw 'Cargo build/test failed' }
