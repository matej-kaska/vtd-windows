param([switch]$Test, [string]$BuildDir = "$env:SystemDrive\vtd-build", [string]$NativeBuildDir = "$env:SystemDrive\vtd-transcribe-build")
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
$env:VSLANG = '1033'
$env:PATH = "$vs\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin;$vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja;$env:PATH"
$env:CMAKE_GENERATOR = 'Ninja'
$env:CARGO_TARGET_DIR = $BuildDir
if (Test-Path "$root\.tools\libclang\clang\native\libclang.dll") { $env:LIBCLANG_PATH = "$root\.tools\libclang\clang\native" }
Remove-Item Env:\WHISPER_DONT_GENERATE_BINDINGS -ErrorAction SilentlyContinue
$env:CMAKE_BUILD_PARALLEL_LEVEL = '2'
$env:CARGO_BUILD_JOBS = '2'
$env:GGML_NATIVE = 'OFF'
& "$PSScriptRoot\build-transcribe.ps1" -BuildDir $NativeBuildDir
$env:CMAKE_PROJECT_INCLUDE = "$root\scripts\whisper-windows.cmake".Replace('\', '/')
$nativeHash = ((Get-FileHash "$root\patches\windows\whisper.patch", "$root\scripts\whisper-windows.cmake" -Algorithm SHA256).Hash) -join ''
$nativeStamp = Join-Path $BuildDir 'vtd-native.sha256'
if (-not (Test-Path $nativeStamp) -or (Get-Content $nativeStamp -Raw) -ne $nativeHash) {
    cargo clean --release -p whisper-rs-sys
    if ($LASTEXITCODE -ne 0) { throw 'Cannot refresh native build' }
    New-Item -ItemType Directory -Force $BuildDir | Out-Null
    [IO.File]::WriteAllText($nativeStamp, $nativeHash)
}
if ($Test) {
    cargo test --release --locked --features engine
    if ($LASTEXITCODE -ne 0) { throw 'Whisper tests failed' }
    cargo test --release --locked --features transcribe-engine --bin vtd-transcribe
} else {
    cargo rustc --release --locked --features engine --bin vtd-engine -- -C lto=fat -C codegen-units=1 -C panic=abort -C link-arg=/DELAYLOAD:Cabinet.dll -C link-arg=/DELAYLOAD:vulkan-1.dll -C link-arg=delayimp.lib
    if ($LASTEXITCODE -ne 0) { throw 'Engine build failed' }
    cargo rustc --release --locked --features transcribe-engine --bin vtd-transcribe -- -C lto=fat -C codegen-units=1 -C panic=abort -C link-arg=/DELAYLOAD:Cabinet.dll -C link-arg=/DELAYLOAD:vulkan-1.dll -C link-arg=delayimp.lib
    if ($LASTEXITCODE -ne 0) { throw 'Transcribe engine build failed' }
    cargo rustc --release --locked --bin vtd -- -C opt-level=s -C lto=fat -C codegen-units=1 -C panic=abort -C link-arg=/DELAYLOAD:comctl32.dll -C link-arg=/DELAYLOAD:ole32.dll -C link-arg=/DELAYLOAD:oleaut32.dll -C link-arg=/DELAYLOAD:combase.dll -C link-arg=delayimp.lib
}
if ($LASTEXITCODE -ne 0) { throw 'Cargo build/test failed' }
