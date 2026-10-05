param([string]$BuildDir = "$env:SystemDrive\vtd-build", [switch]$RealModelDownload, [string]$ReduxModel)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$testRoot = Join-Path $root ('artifacts\installer-test-' + [guid]::NewGuid().ToString('N'))
$httpRoot = Join-Path $testRoot 'http'
$bundle = Join-Path $httpRoot 'bundle'
$downloadDir = Join-Path $testRoot 'download'
New-Item -ItemType Directory -Force $httpRoot,$downloadDir | Out-Null
$fixture = [byte[]]::new(4096)
$fixture[0]=71; $fixture[1]=71; $fixture[2]=85; $fixture[3]=70
for ($i=4; $i -lt $fixture.Length; $i++) { $fixture[$i] = $i % 251 }
[IO.File]::WriteAllBytes((Join-Path $httpRoot 'model.bin'),$fixture)
[IO.File]::WriteAllText((Join-Path $httpRoot 'invalid.bin'),'<html>This is not a speech model.</html>')
$modelHash = (Get-FileHash (Join-Path $httpRoot 'model.bin') -Algorithm SHA256).Hash.ToLowerInvariant()
$server = Start-Process -FilePath (Get-Command python).Source -ArgumentList @('-u',('"' + (Join-Path $PSScriptRoot 'installer-fixtures.py') + '"'),('"' + $httpRoot + '"')) -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $testRoot 'server-error.log')
$cases = [Collections.Generic.List[object]]::new()
$targets = [Collections.Generic.List[string]]::new()
$setup = Join-Path $downloadDir 'VTD-Setup.exe'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$productKey = 'HKCU:\Software\VTD-Installer-Tests'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\VTD-Installer-Tests'

function Assert-True([bool]$Condition,[string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Install-Case([string]$Name,[string]$Destination,[hashtable]$Options,[bool]$Success=$true,[string]$ExpectedError='') {
    $ini = Join-Path $testRoot "$Name.ini"
    $settings = @{Launch='0';Autostart='0'}
    if (-not (Test-Path (Join-Path $Destination 'vtd.json'))) { $settings.Language='cs'; $settings.ModelPreset='canary' }
    foreach ($key in $Options.Keys) { $settings[$key] = $Options[$key] }
    $text = "[Settings]`r`n" + (($settings.Keys | Sort-Object | ForEach-Object { "$_=$($settings[$_])" }) -join "`r`n") + "`r`n"
    [IO.File]::WriteAllText($ini,$text,[Text.Encoding]::Unicode)
    $statusFile = Join-Path $downloadDir 'test-status.ini'
    if (Test-Path -LiteralPath $statusFile) { Remove-Item -LiteralPath $statusFile }
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $setup -ArgumentList ('/S /OPTIONS="' + $ini + '" /D=' + $Destination) -WindowStyle Hidden -PassThru
    $timeout = if ($RealModelDownload) { 600000 } else { 180000 }
    if (-not $process.WaitForExit($timeout)) { $process.Kill(); throw "$Name timed out" }
    $process.Refresh()
    $status = Get-Content -LiteralPath $statusFile -Raw -ErrorAction SilentlyContinue
    Assert-True (($process.ExitCode -eq 0) -eq $Success) "$Name returned $($process.ExitCode). $status"
    if ($ExpectedError) { Assert-True ($status.Contains($ExpectedError)) "$Name failed for an unexpected reason: $status" }
    $cases.Add([ordered]@{Case=$Name;ExitCode=$process.ExitCode;Seconds=[math]::Round($timer.Elapsed.TotalSeconds,2)})
    Write-Output "$Name : passed (exit $($process.ExitCode))"
    if ($Success -and -not $targets.Contains($Destination)) { $targets.Add($Destination) }
}

function Uninstall-Case([string]$Destination,[bool]$Purge=$false) {
    $binary = Join-Path $Destination 'Uninstall.exe'
    if (-not (Test-Path -LiteralPath $binary)) { return }
    # _?= avoids the usual detached self-copy, so we can wait for actual completion.
    $copy = Join-Path $testRoot ('uninstall-' + [guid]::NewGuid().ToString('N') + '.exe')
    Copy-Item -LiteralPath $binary -Destination $copy
    $arguments = '/S ' + $(if ($Purge) { '/PURGE ' } else { '' }) + '_?=' + $Destination
    $process = Start-Process -FilePath $copy -ArgumentList $arguments -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(30000)) { $process.Kill(); throw 'Uninstaller timed out' }
    Assert-True ($process.ExitCode -eq 0) "Uninstall failed: $($process.ExitCode)"
}

try {
    for ($i=0; $i -lt 30 -and -not (Test-Path (Join-Path $httpRoot 'server.json')); $i++) { Start-Sleep -Milliseconds 100 }
    $baseUrl = (Get-Content (Join-Path $httpRoot 'server.json') -Raw | ConvertFrom-Json).url
    $buildArgs = @{BuildDir=$BuildDir;OutputDir=$bundle;PayloadUrl="$baseUrl/bundle/vtd-runtime-x64.zip";TestHarness=$true}
    if (-not $RealModelDownload) { $buildArgs.TestModelUrl="$baseUrl/model.bin"; $buildArgs.TestModelHash=$modelHash }
    & "$PSScriptRoot\build-installer.ps1" @buildArgs | Out-File (Join-Path $testRoot 'build.log')
    Copy-Item -LiteralPath (Join-Path $bundle 'VTD-Setup.exe') -Destination $setup
    $target = Join-Path $testRoot 'installed with spaces'
    foreach ($invalid in @('0','27','91','160','229','231','4096')) {
        Install-Case "invalid-shortcut-$invalid" $target @{HoldKey=$invalid;ModelSource='default'} $false 'Choose a key, optionally with Ctrl, Shift or Alt.'
    }
    Install-Case 'duplicate-shortcuts' $target @{HoldKey='120';ToggleKey='120';ModelSource='default'} $false 'Each action must use a different key.'
    Install-Case 'invalid-preset' $target @{ModelSource='default';ModelPreset='unknown'} $false 'Choose a valid model preset.'
    Install-Case 'canary-rejects-auto' $target @{ModelSource='default';ModelPreset='canary';Language='auto'} $false 'This model does not support'
    Install-Case 'redux-rejects-japanese' $target @{ModelSource='default';ModelPreset='redux';Language='ja'} $false 'This model does not support'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $target 'vtd.exe'))) 'Invalid preferences installed application files'

    $payload = Join-Path $bundle 'vtd-runtime-x64.zip'
    $originalPayload = [IO.File]::ReadAllBytes($payload)
    [IO.File]::WriteAllText($payload,'corrupted application payload')
    Install-Case 'payload-hash-failure' $target @{ModelSource='default'} $false 'Application SHA-256 mismatch.'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $target 'vtd.exe'))) 'Bad payload installed application files'
    [IO.File]::WriteAllBytes($payload,$originalPayload)
    $sidecar = Join-Path $downloadDir 'vtd-runtime-x64.zip'
    [IO.File]::WriteAllText($sidecar,'wrong adjacent payload')
    Install-Case 'adjacent-payload-hash-failure' $target @{ModelSource='default'} $false 'The adjacent application ZIP does not match this installer.'
    Remove-Item -LiteralPath $sidecar

    Install-Case 'default-download' $target @{HoldKey='122';ToggleKey='123';ReplayKey='124';Language='en';Mute='1';Autostart='1';ModelSource='default'}
    $configPath = Join-Path $target 'vtd.json'
    $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-True ($config.trigger_key -eq 122 -and $config.toggle_key -eq 123 -and $config.replay_key -eq 124 -and $config.language -eq 'en' -and $config.mute_output) 'Preferences were not saved'
    $expectedModelHash = if ($RealModelDownload) { '49e0a67e219bec95a254c2348460b6350e75a7ac6f93a131e48244b4c7cb53b9' } else { $modelHash }
    Assert-True ((Get-FileHash (Join-Path $target $config.model)).Hash -eq $expectedModelHash) 'Default model hash differs'
    Assert-True ((Get-ItemPropertyValue $runKey 'VTD Installer Test') -eq ('"' + (Join-Path $target 'vtd.exe') + '" run')) 'Autostart entry differs'
    Assert-True ((Get-ChildItem -LiteralPath $target -Filter *.ps1).Count -eq 0) 'Installed runtime contains PowerShell scripts'
    if ($ReduxModel) {
        $redux = (Get-Content "$root/assets/models.json" -Raw -Encoding UTF8 | ConvertFrom-Json).models | Where-Object id -eq 'redux'
        Assert-True ((Get-FileHash -LiteralPath $ReduxModel).Hash -eq $redux.sha256) 'Redux fixture hash differs'
        Copy-Item -LiteralPath $ReduxModel -Destination (Join-Path "$target/models" $redux.file)
        foreach ($language in @('cs','auto')) {
            Install-Case "redux-preset-$language" $target @{ModelSource='default';ModelPreset='redux';Language=$language}
            $saved = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
            Assert-True ($saved.model -eq ('models\' + $redux.file) -and $saved.language -eq $language) 'Redux preset or language was not applied'
            Assert-True ($saved.trigger_key -eq 122 -and $saved.mute_output) 'Redux selection changed unrelated preferences'
        }
        Install-Case 'restore-canary-after-redux' $target @{ModelSource='default';ModelPreset='canary';Language='en'}
    }
    $requestsPath = Join-Path $httpRoot 'requests.log'
    $modelRequests = @(Select-String -Path $requestsPath -SimpleMatch 'GET /model.bin').Count
    Install-Case 'custom-shortcuts' $target @{HoldKey='800';ToggleKey='173';ReplayKey='544';ModelSource='default'}
    $customKeys = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-True ($customKeys.trigger_key -eq 800 -and $customKeys.toggle_key -eq 173 -and $customKeys.replay_key -eq 544) 'Custom shortcuts were not saved'
    Install-Case 'custom-shortcuts-upgrade' $target @{}
    $customKeys = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-True ($customKeys.trigger_key -eq 800 -and $customKeys.toggle_key -eq 173 -and $customKeys.replay_key -eq 544) 'Upgrade changed custom shortcuts'
    Install-Case 'reuse-verified-default-model' $target @{ModelSource='default'}
    Assert-True (@(Select-String -Path $requestsPath -SimpleMatch 'GET /model.bin').Count -eq $modelRequests) 'Matching default model was downloaded again'

    # Test UTF-8 rather than the machine's ANSI code page, including preservation
    # of fields not shown by the installer, and a relative model path.
    $config | Add-Member -NotePropertyName microphone -NotePropertyValue ([string][char]0x010D + 'esky mikrofon ' + [char]0x65E5) -Force
    $config | Add-Member -NotePropertyName threads -NotePropertyValue 7 -Force
    [IO.File]::WriteAllText($configPath,($config | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
    $before = [IO.File]::ReadAllBytes($configPath)
    Install-Case 'model-hash-failure' $target @{ModelSource='url';ModelUrl="$baseUrl/model.bin";ModelHash=('0'*64)} $false 'Model SHA-256 mismatch.'
    Assert-True ([Convert]::ToBase64String([IO.File]::ReadAllBytes($configPath)) -eq [Convert]::ToBase64String($before)) 'Failed download modified the existing config'
    Install-Case 'invalid-model-header' $target @{ModelSource='url';ModelUrl="$baseUrl/invalid.bin";ModelHash=''} $false 'This file is not a compatible GGML or GGUF speech model.'
    Install-Case 'missing-model-url' $target @{ModelSource='url';ModelUrl="$baseUrl/not-found.bin";ModelHash=''} $false 'Model download failed:'
    $runtimeFiles = @('vtd.exe','vtd-helper.exe','vtd-engine.exe','vtd-transcribe.exe','msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll','vtd.json','Uninstall.exe')
    $beforeHashes = @{}
    foreach ($name in $runtimeFiles) { $beforeHashes[$name] = (Get-FileHash (Join-Path $target $name)).Hash }
    $lockedFile = [IO.File]::Open((Join-Path $target 'msvcp140.dll'),[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    try { Install-Case 'locked-file-rolls-back-upgrade' $target @{HoldKey='125'} $false 'Cannot write the installation.' }
    finally { $lockedFile.Dispose() }
    foreach ($name in $runtimeFiles) { Assert-True ((Get-FileHash (Join-Path $target $name)).Hash -eq $beforeHashes[$name]) "Rollback changed $name" }
    Install-Case 'upgrade-preserves-preferences' $target @{}
    $updated = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-True ($updated.microphone -ceq $config.microphone -and $updated.threads -eq 7 -and $updated.model -eq $config.model -and $updated.trigger_key -eq 122 -and $updated.mute_output) 'Upgrade changed existing preferences or Unicode values'
    Uninstall-Case $target
    Assert-True ((Test-Path -LiteralPath $configPath) -and (Test-Path -LiteralPath (Join-Path $target $config.model))) 'Uninstall removed retained settings/model'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $target 'vtd.exe'))) 'Uninstall kept the executable'
    $cases.Add([ordered]@{Case='uninstall-keeps-model-and-config';ExitCode=0})

    # Verify the other presets offline against real, hashed local models when available.
    $catalog = Get-Content "$root\assets\models.json" -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($preset in $catalog.models | Where-Object { $_.id -ne 'canary' }) {
        $source = Join-Path $root $(if ($preset.id -eq 'whisper') { 'models\' + $preset.file } else { 'artifacts\asr-comparison\models\' + $preset.file })
        if (-not (Test-Path -LiteralPath $source)) { continue }
        $destination = Join-Path $testRoot ('preset-' + $preset.id)
        $modelDir = Join-Path $destination 'models'
        New-Item -ItemType Directory -Force $modelDir | Out-Null
        New-Item -ItemType HardLink -Path (Join-Path $modelDir $preset.file) -Target $source | Out-Null
        Install-Case ('preset-' + $preset.id) $destination @{ModelSource='default';ModelPreset=$preset.id;Language='auto'}
        $savedPreset = Get-Content (Join-Path $destination 'vtd.json') -Raw -Encoding UTF8 | ConvertFrom-Json
        Assert-True ($savedPreset.model -eq ('models\' + $preset.file)) 'Wrong preset model path'
        Install-Case ('upgrade-' + $preset.id) $destination @{}
        $upgradedPreset = Get-Content (Join-Path $destination 'vtd.json') -Raw -Encoding UTF8 | ConvertFrom-Json
        Assert-True ($upgradedPreset.model -eq $savedPreset.model -and $upgradedPreset.language -eq 'auto') 'Upgrade changed model or language'
        Uninstall-Case $destination $true
        Assert-True (-not (Test-Path (Join-Path $modelDir $preset.file))) 'Purge kept a managed preset'
        Assert-True (Test-Path -LiteralPath $source) 'Purge removed source model'
    }

    $custom = Join-Path $testRoot 'custom model download'
    Install-Case 'custom-url-and-hash' $custom @{ModelSource='url';ModelUrl="$baseUrl/model.bin";ModelHash=$modelHash}
    Assert-True ((Get-FileHash (Join-Path $custom 'models\custom-model.bin')).Hash -eq $modelHash) 'Custom model differs'
    [IO.File]::WriteAllText((Join-Path $custom 'unrelated.txt'),'keep this file')
    Uninstall-Case $custom $true
    Assert-True (-not (Test-Path (Join-Path $custom 'models\custom-model.bin')) -and (Test-Path (Join-Path $custom 'unrelated.txt'))) 'Purge did not remove only its owned data'
    $cases.Add([ordered]@{Case='uninstall-purge-keeps-unrelated-files';ExitCode=0})

    $external = Join-Path $httpRoot ([string][char]0x017D + 'lu' + [char]0x0165 + 'ou' + [char]0x010D + 'ky model.bin')
    Copy-Item (Join-Path $httpRoot 'model.bin') -Destination $external
    $existing = Join-Path $testRoot ([string][char]0x010C + 'eska instalace')
    Copy-Item -LiteralPath $payload -Destination $sidecar
    $server.Kill(); $server.WaitForExit()
    Install-Case 'existing-unicode-model-path' $existing @{ModelSource='file';ModelFile=$external}
    $saved = Get-Content (Join-Path $existing 'vtd.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-True ($saved.model -ceq $external) 'Unicode model path was corrupted'
    Uninstall-Case $existing $true
    Assert-True (Test-Path -LiteralPath $external) 'Uninstall deleted an external model'
    $cases.Add([ordered]@{Case='uninstall-keeps-external-model';ExitCode=0})
    $cases | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $testRoot 'results.json') -Encoding UTF8
    Write-Output "Verified $($cases.Count) installer cases. Evidence: $testRoot"
} finally {
    foreach ($destination in $targets) { Uninstall-Case $destination }
    if (-not $server.HasExited) { $server.Kill(); $server.WaitForExit() }
}
