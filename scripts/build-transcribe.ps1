param([string]$BuildDir = "$env:SystemDrive\vtd-transcribe-build")
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$source = Join-Path $root '.tools\transcribe.cpp'
$revision = 'e85b30edac87533168863283c1e595bf39bd7d15'
if (-not (Test-Path "$source\.git")) {
    git clone --no-checkout https://github.com/handy-computer/transcribe.cpp.git $source
    if ($LASTEXITCODE -ne 0) { throw 'Cannot fetch transcribe.cpp' }
    git -C $source checkout --detach $revision
    if ($LASTEXITCODE -ne 0) { throw 'Cannot check out pinned transcribe.cpp' }
}
if ((git -C $source rev-parse HEAD) -ne $revision) { throw 'Unexpected transcribe.cpp revision; keep the pinned checkout.' }
$patch = Join-Path $root 'patches\windows\transcribe.patch'
if (Test-Path $patch) {
    $previousErrorPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    git -C $source apply --reverse --check $patch 2>$null
    $applied = $LASTEXITCODE -eq 0
    $ErrorActionPreference = $previousErrorPreference
    if (-not $applied) {
        git -C $source apply --check $patch
        if ($LASTEXITCODE -ne 0) { throw 'transcribe.cpp patch does not apply' }
        git -C $source apply $patch
        if ($LASTEXITCODE -ne 0) { throw 'Cannot patch transcribe.cpp' }
    }
}
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -property installationPath
$env:PATH = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer;$env:PATH"
$devcmd = '"' + $vs + '\Common7\Tools\VsDevCmd.bat" -arch=x64 -host_arch=x64 >nul && set'
cmd /c $devcmd | ForEach-Object { if ($_ -match '^([^=]+)=(.*)$') { [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process') } }
$env:VSLANG = '1033'
$env:PATH = "$vs\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin;$vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja;$env:PATH"
if (Test-Path "$root\.tools\vulkan\Bin\glslc.exe") { $env:VULKAN_SDK = "$root\.tools\vulkan" }
if (-not $env:VULKAN_SDK) { throw 'Install Vulkan SDK first.' }
Remove-Item Env:CMAKE_PROJECT_INCLUDE -ErrorAction SilentlyContinue
cmake -S $source -B $BuildDir -G Ninja -DCMAKE_BUILD_TYPE=Release -DTRANSCRIBE_VULKAN=ON -DTRANSCRIBE_BUILD_SHARED=OFF -DTRANSCRIBE_BUILD_TESTS=OFF -DTRANSCRIBE_BUILD_EXAMPLES=OFF -DTRANSCRIBE_BUILD_TOOLS=OFF -DTRANSCRIBE_USE_SYSTEM_BLAS=OFF -DGGML_NATIVE=OFF "-DCMAKE_ARCHIVE_OUTPUT_DIRECTORY=$BuildDir/lib" "-DVTD_ROOT=$($root.Replace('\','/'))" "-DCMAKE_PROJECT_INCLUDE=$($root.Replace('\','/'))/scripts/transcribe-windows.cmake"
if ($LASTEXITCODE -ne 0) { throw 'Native engine configuration failed' }
cmake --build $BuildDir --target vtd-transcribe-bridge --parallel 2
if ($LASTEXITCODE -ne 0) { throw 'Native engine build failed' }
$env:VTD_TRANSCRIBE_LIB_DIR = "$BuildDir\lib"
