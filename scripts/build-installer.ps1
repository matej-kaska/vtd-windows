param(
    [string]$BuildDir = "$env:SystemDrive\vtd-build",
    [string]$ReleaseTag,
    [string]$Repository = 'matej-kaska/vtd-windows',
    [string]$PayloadUrl,
    [string]$OutputDir,
    [ValidateSet('lzma','zlib')][string]$Compression = 'lzma',
    [switch]$TestHarness,
    [string]$TestModelUrl,
    [string]$TestModelHash
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$output = if ($OutputDir) { [IO.Path]::GetFullPath($OutputDir) } else { Join-Path $root 'dist\installer' }
$version = [regex]::Match([IO.File]::ReadAllText((Join-Path $root 'Cargo.toml')), '(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"').Groups[1].Value
if (-not $ReleaseTag) { $ReleaseTag = "v$version" }
if ($ReleaseTag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?$') { throw 'Use a version tag such as v0.3.0' }
if ($Repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') { throw 'Invalid GitHub repository' }
if (-not $PayloadUrl) { $PayloadUrl = "https://github.com/$Repository/releases/download/$ReleaseTag/vtd-runtime-x64.zip" }
$uri = [uri]$PayloadUrl
if (-not $uri.IsAbsoluteUri -or ($uri.Scheme -ne 'https' -and -not ($TestHarness -and $uri.Scheme -eq 'http' -and $uri.IsLoopback))) { throw 'Payload URL must use HTTPS (loopback HTTP is allowed only for test builds)' }
if ($PayloadUrl -match '["$\r\n]') { throw 'Unsupported characters in payload URL' }
if ($TestModelUrl) {
    $modelUri = [uri]$TestModelUrl
    if (-not $TestHarness -or -not $modelUri.IsLoopback -or $TestModelUrl -match '["$\r\n]' -or $TestModelHash -notmatch '^[a-fA-F0-9]{64}$') { throw 'Model fixtures require a test build, a loopback URL and SHA256' }
}
New-Item -ItemType Directory -Force $output | Out-Null
$tools = Join-Path $root '.tools\installer'
$compiler = Join-Path $tools 'nsis-3.13\makensis.exe'
$requiredTools = @($compiler, (Join-Path $tools 'inetc\Plugins\x86-unicode\INetC.dll'), (Join-Path $tools 'nsjson\Plugins\x86-unicode\nsJSON.dll'))
if (@($requiredTools | Where-Object { -not (Test-Path -LiteralPath $_) }).Count) { & "$PSScriptRoot\setup-installer.ps1" }

# A strict runtime manifest: no model, personal config, PowerShell, SDK, README,
# unused CRT DLLs, or installer plugins in the installed application's payload.
$stage = Join-Path $output 'runtime'
New-Item -ItemType Directory -Force $stage | Out-Null
$files = @('vtd.exe','vtd-engine.exe','msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll','LICENSE','THIRD_PARTY_LICENSES.txt','INSTALLER_LICENSES.txt')
foreach ($name in @('vtd.exe','vtd-engine.exe')) { Copy-Item -LiteralPath (Join-Path "$BuildDir\release" $name) -Destination $stage }
foreach ($name in @('LICENSE','THIRD_PARTY_LICENSES.txt')) { Copy-Item -LiteralPath (Join-Path $root $name) -Destination $stage }
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -property installationPath
$crt = Get-Item "$vs\VC\Redist\MSVC\*\x64\Microsoft.VC143.CRT" | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $crt) { throw 'MSVC runtime not found' }
foreach ($name in @('msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll')) { Copy-Item -LiteralPath (Join-Path $crt.FullName $name) -Destination $stage }
$nsjsonReadme = [IO.File]::ReadAllText((Join-Path $tools 'nsjson\Docs\nsJSON\Readme.txt'))
$notices = "Installer components (not loaded by VTD):`r`nNSIS 3.13: https://nsis.sourceforge.io/`r`nINetC: Copyright (c) 2004-2015 Takhir Bedertdinov and NSIS contributors (NSIS license).`r`nnsJSON 1.1.1.0: Stuart Welch, https://nsis.sourceforge.io/NsJSON_plug-in`r`n`r`n"
$notices += [IO.File]::ReadAllText((Join-Path $tools 'nsis-3.13\COPYING'))
$notices += "`r`nnsJSON license:`r`n" + $nsjsonReadme.Substring($nsjsonReadme.LastIndexOf("License"))
[IO.File]::WriteAllText((Join-Path $stage 'INSTALLER_LICENSES.txt'), $notices)

Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$archive = Join-Path $output 'vtd-runtime-x64.zip'
$sevenZip = (Get-Command 7z -ErrorAction SilentlyContinue).Source
if (-not $sevenZip -and (Test-Path "$env:ProgramFiles\7-Zip\7z.exe")) { $sevenZip = "$env:ProgramFiles\7-Zip\7z.exe" }
if ($sevenZip) {
    if (Test-Path -LiteralPath "$archive.tmp") { Remove-Item -LiteralPath "$archive.tmp" }
    foreach ($name in $files) { (Get-Item -LiteralPath (Join-Path $stage $name)).LastWriteTime = [datetime]'2020-01-01T00:00:00' }
    Push-Location $stage
    try {
        & $sevenZip a -tzip -mm=Deflate -mx=9 -mfb=258 -mpass=15 -mmt=1 -mtc=off -mta=off "$archive.tmp" @files | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'Runtime ZIP compression failed' }
    } finally { Pop-Location }
} else {
    $zip = [IO.Compression.ZipArchive]::new([IO.File]::Create("$archive.tmp"), [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($name in $files) {
            $entry = $zip.CreateEntry($name,[IO.Compression.CompressionLevel]::Optimal)
            $entry.LastWriteTime = [DateTimeOffset]::new(2020,1,1,0,0,0,[TimeSpan]::Zero)
            $input = [IO.File]::OpenRead((Join-Path $stage $name))
            $stream = $entry.Open()
            try { $input.CopyTo($stream) } finally { $stream.Dispose(); $input.Dispose() }
        }
    }
    finally { $zip.Dispose() }
}
Move-Item -LiteralPath "$archive.tmp" -Destination $archive -Force
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()

# Generate the installer language list from the application's authoritative table.
$languageSource = [IO.File]::ReadAllText((Join-Path $root 'src\windows\languages.rs'))
$languages = [regex]::Matches($languageSource, '(?m)^    \("([a-z]+)", "([^"]+)"\),')
if ($languages.Count -ne 101) { throw 'Unexpected speech language list' }
$generated = [Text.StringBuilder]::new()
[void]$generated.AppendLine('!macro LanguageItems CONTROL')
foreach ($language in $languages) { [void]$generated.AppendLine('  ${NSD_CB_AddString} ${CONTROL} "' + $language.Groups[2].Value + '"') }
[void]$generated.AppendLine('!macroend')
[void]$generated.AppendLine('Function LanguageName')
[void]$generated.AppendLine('  StrCpy $LangName "Detect from speech"')
foreach ($language in $languages) { [void]$generated.AppendLine('  ${If} $SpeechLanguage == "' + $language.Groups[1].Value + '"'); [void]$generated.AppendLine('    StrCpy $LangName "' + $language.Groups[2].Value + '"'); [void]$generated.AppendLine('  ${EndIf}') }
[void]$generated.AppendLine('FunctionEnd')
[void]$generated.AppendLine('Function LanguageCode')
[void]$generated.AppendLine('  StrCpy $SpeechLanguage ""')
foreach ($language in $languages) { [void]$generated.AppendLine('  ${If} $LangName == "' + $language.Groups[2].Value + '"'); [void]$generated.AppendLine('    StrCpy $SpeechLanguage "' + $language.Groups[1].Value + '"'); [void]$generated.AppendLine('  ${EndIf}') }
[void]$generated.AppendLine('FunctionEnd')
[IO.File]::WriteAllText((Join-Path $output 'languages.nsh'),$generated.ToString())
$installedKiB = [math]::Ceiling((($files | ForEach-Object { (Get-Item (Join-Path $stage $_)).Length } | Measure-Object -Sum).Sum + 128KB) / 1KB)
$args = @('/V3',"/DCOMPRESSION=$Compression","/DOUTPUT_DIR=$output","/DTOOLS_DIR=$tools","/DAPP_VERSION=$($ReleaseTag.Substring(1))","/DVERSION_NUMBER=$($ReleaseTag.Substring(1).Split('-')[0]).0","/DPAYLOAD_URL=$PayloadUrl","/DPAYLOAD_SHA256=$hash","/DINSTALLED_KIB=$installedKiB")
if ($TestHarness) { $args += '/DTEST_HARNESS' }
if ($TestModelUrl) { $args += "/DDEFAULT_URL=$TestModelUrl", "/DDEFAULT_SHA=$TestModelHash" }
& $compiler @args (Join-Path $root 'installer\vtd.nsi')
if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
$setup = Join-Path $output 'VTD-Setup.exe'
$report = [ordered]@{Version=$ReleaseTag;SetupBytes=(Get-Item $setup).Length;PayloadBytes=(Get-Item $archive).Length;PayloadSha256=$hash;PayloadUrl=$PayloadUrl;ModelBundled=$false;Compression=$Compression;RuntimeFiles=@($files | ForEach-Object { [ordered]@{Name=$_;Bytes=(Get-Item (Join-Path $stage $_)).Length} })}
if ($report.SetupBytes -gt 300KB) { throw 'Installer exceeded the 300 KiB size budget' }
$report | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $output 'size-report.json') -Encoding UTF8
@($setup,$archive) | ForEach-Object { ((Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant()) + '  ' + [IO.Path]::GetFileName($_) } | Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS.txt') -Encoding ASCII
$report | ConvertTo-Json -Depth 4
