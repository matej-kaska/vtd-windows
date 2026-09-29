$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
New-Item -ItemType Directory -Force "$root\.tools" | Out-Null
$sdk = "$root\.tools\vulkan-sdk.exe"
$hash = '94a82d378f7a5e3e54c9db7d2fb7016af136e14ac0a18dbf0f2f67a36352d141'
$stamp = "$root\.tools\vulkan\.vtd-sdk"
$ready = (Test-Path $stamp) -and (Get-Content $stamp -Raw) -eq $hash
foreach ($name in @('Bin\glslc.exe', 'Lib\vulkan-1.lib', 'Include\vulkan\vulkan.hpp')) {
    $ready = $ready -and (Test-Path "$root\.tools\vulkan\$name")
}
if (-not $ready) {
    if (-not (Test-Path $sdk) -or (Get-FileHash $sdk -Algorithm SHA256).Hash -ne $hash) {
        curl.exe -L --fail --retry 3 --silent --show-error 'https://sdk.lunarg.com/sdk/download/1.4.363.0/windows/vulkansdk-windows-X64-1.4.363.0.exe' -o $sdk
        if ($LASTEXITCODE -ne 0) { throw 'Vulkan SDK download failed' }
    }
    if ((Get-FileHash $sdk -Algorithm SHA256).Hash -ne $hash) { throw 'Vulkan SDK SHA256 mismatch' }
    python "$PSScriptRoot\extract-sdk.py" $sdk "$root\.tools\vulkan" 'C:\Program Files\7-Zip\7z.exe'
    if ($LASTEXITCODE -ne 0) { throw 'Vulkan SDK extraction failed' }
    [IO.File]::WriteAllText($stamp, $hash)
    Remove-Item -LiteralPath $sdk
}
if (-not (Test-Path "$root\.tools\libclang\clang\native\libclang.dll") -or -not (Test-Path "$root\.tools\libclang\libclang-18.1.1.dist-info")) {
    python -m pip install --disable-pip-version-check --target "$root\.tools\libclang" libclang==18.1.1
    if ($LASTEXITCODE -ne 0) { throw 'libclang installation failed' }
}
