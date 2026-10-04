param(
    [string]$BuildDir = "$env:SystemDrive\vtd-build",
    [switch]$Test,
    [switch]$ResidentTest
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
if (Test-Path "$root\.tools\cargo\bin\cargo.exe") {
    $env:CARGO_HOME = "$root\.tools\cargo"
    $env:RUSTUP_HOME = "$root\.tools\rustup"
    $env:PATH = "$root\.tools\cargo\bin;$env:PATH"
}
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -property installationPath
$env:PATH = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer;$env:PATH"
$devcmd = '"' + $vs + '\Common7\Tools\VsDevCmd.bat" -arch=x64 -host_arch=x64 >nul && set'
cmd /c $devcmd | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') {
        [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process')
    }
}
$env:VSLANG = '1033'
$env:CARGO_TARGET_DIR = $BuildDir
if ($Test) {
    cargo test --release --locked --features windows-helper --bin vtd-helper --lib
} else {
    $helperFeatures = if ($ResidentTest) { 'windows-helper,resident-test' } else { 'windows-helper' }
    cargo rustc --release --locked --features $helperFeatures --bin vtd-helper -- -C opt-level=s -C lto=fat -C codegen-units=1 -C panic=abort -C link-arg=/DELAYLOAD:comctl32.dll -C link-arg=/DELAYLOAD:ole32.dll -C link-arg=/DELAYLOAD:oleaut32.dll -C link-arg=/DELAYLOAD:combase.dll -C link-arg=delayimp.lib
    if ($LASTEXITCODE -ne 0) { throw 'Helper build failed' }
    $residentFeatures = if ($ResidentTest) { @('--features', 'resident-test') } else { @() }
    cargo rustc --release --locked --bin vtd @residentFeatures -- -C opt-level=z -C lto=fat -C codegen-units=1 -C panic=abort
}
if ($LASTEXITCODE -ne 0) { throw 'Tray build/test failed' }
