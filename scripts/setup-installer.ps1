$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$tools = Join-Path $root '.tools\installer'
New-Item -ItemType Directory -Force $tools | Out-Null
# Portable build tools only. Nothing is installed in Windows or needed by users.
$downloads = @(
    @('nsis-3.13', 'https://downloads.sourceforge.net/project/nsis/NSIS%203/3.13/nsis-3.13.zip', 'ba63dffc4410ee89193e1cb5a41989991bd77c61068da17e3156d136b7b0b3d8', $tools),
    @('inetc', 'https://nsis.sourceforge.io/mediawiki/images/c/c9/Inetc.zip', '88d9dcffbe967df6ee9f5820f8199a673383e581bf650dbc35b94846314b1a4b', (Join-Path $tools 'inetc')),
    @('nsjson', 'https://nsis.sourceforge.io/mediawiki/images/f/f0/NsJSON.zip', 'a3422c34509ddd67f2564d256d3284b4d3be18ee5fe21319a0fcdb4e8e2b2861', (Join-Path $tools 'nsjson'))
)
foreach ($entry in $downloads) {
    $name, $url, $hash, $destination = $entry
    $archive = Join-Path $tools "$name.zip"
    if (-not (Test-Path -LiteralPath $archive) -or (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $hash) {
        curl.exe -L --fail --retry 3 --silent --show-error $url -o $archive
        if ($LASTEXITCODE -ne 0) { throw "Cannot download $name" }
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $hash) { throw "Invalid $name SHA256" }
    Expand-Archive -LiteralPath $archive -DestinationPath $destination -Force
}
