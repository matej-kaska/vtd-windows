param(
    [ValidateSet('q5_0','q8_0','f16')][string]$Quality = 'q5_0',
    [string]$Destination = (Join-Path (Split-Path $PSScriptRoot -Parent) 'models')
)
$ErrorActionPreference = 'Stop'
$models = @{
    q5_0 = @('ggml-large-v3-turbo-q5_0.bin', '394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2')
    q8_0 = @('ggml-large-v3-turbo-q8_0.bin', '317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1')
    f16 = @('ggml-large-v3-turbo.bin', '1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69')
}
$name, $hash = $models[$Quality]
New-Item -ItemType Directory -Force $Destination | Out-Null
$path = Join-Path $Destination $name
if ((Test-Path $path) -and (Get-FileHash $path -Algorithm SHA256).Hash -eq $hash) { Write-Output $path; exit 0 }
curl.exe -L --fail --retry 3 --silent --show-error "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/$name" -o "$path.part"
if ($LASTEXITCODE -ne 0) { throw 'Model download failed' }
if ((Get-FileHash "$path.part" -Algorithm SHA256).Hash -ne $hash) { throw 'Model SHA256 mismatch' }
Move-Item -LiteralPath "$path.part" -Destination $path -Force
Write-Output $path
