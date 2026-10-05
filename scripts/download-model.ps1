param(
    [ValidateSet('q5_0','q8_0','f16')][string]$Quality = 'q5_0',
    [string]$Destination,
    [ValidateSet('canary','redux','whisper')][string]$Model
)
$ErrorActionPreference = 'Stop'
if (-not $Destination) {
    $base = if (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'vtd.exe')) { $PSScriptRoot } else { Split-Path $PSScriptRoot -Parent }
    $Destination = Join-Path $base 'models'
}
$models = @{
    q5_0 = @('ggml-large-v3-turbo-q5_0.bin', '394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2')
    q8_0 = @('ggml-large-v3-turbo-q8_0.bin', '317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1')
    f16 = @('ggml-large-v3-turbo.bin', '1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69')
}
$name, $hash = $models[$Quality]
$url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/$name"
if ($Model) {
    $catalogPath = if (Test-Path (Join-Path $PSScriptRoot 'models.json')) { Join-Path $PSScriptRoot 'models.json' } else { Join-Path (Split-Path $PSScriptRoot -Parent) 'assets/models.json' }
    $preset = (Get-Content $catalogPath -Raw -Encoding UTF8 | ConvertFrom-Json).models | Where-Object { $_.id -eq $Model }
    $name = $preset.file
    $hash = $preset.sha256
    $url = $preset.url
}
New-Item -ItemType Directory -Force $Destination | Out-Null
$path = Join-Path $Destination $name
if ((Test-Path -LiteralPath $path) -and (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -eq $hash) { Write-Output $path; return }
curl.exe -L --fail --retry 3 --proto '=https' --proto-redir '=https' --progress-bar --show-error $url -o "$path.part"
if ($LASTEXITCODE -ne 0) { throw 'Model download failed' }
if ((Get-FileHash -LiteralPath "$path.part" -Algorithm SHA256).Hash -ne $hash) { throw 'Model SHA256 mismatch' }
Move-Item -LiteralPath "$path.part" -Destination $path -Force
Write-Output $path
