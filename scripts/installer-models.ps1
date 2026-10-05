param([Parameter(Mandatory=$true)][string]$OutputDir)
$root = Split-Path $PSScriptRoot -Parent
$catalog = Get-Content -LiteralPath "$root\assets\models.json" -Raw -Encoding UTF8 | ConvertFrom-Json
$models = $catalog.models
$defines = @(
    '!define DEFAULT_MODEL "' + $models[0].file + '"'
    '!define /ifndef DEFAULT_URL "' + $models[0].url + '"'
    '!define /ifndef DEFAULT_SHA "' + $models[0].sha256 + '"'
    '!define MODEL_BENCHMARK "' + $catalog.benchmark + '"'
)
[IO.File]::WriteAllLines((Join-Path $OutputDir 'model-defines.nsh'), $defines)
$lines = [Collections.Generic.List[string]]::new()
$lines.Add('!macro ModelChoices')
$controls = @('$DefaultControl', '$ModelTwoControl', '$ModelThreeControl')
for ($i=0; $i -lt $models.Count; $i++) {
    $model = $models[$i]
    $title = $model.name + $(if ($i -eq 0) { ' (recommended)' } else { '' })
    $lines.Add('  ${NSD_CreateRadioButton} 0 ' + (2 + 32*$i) + 'u 100% 12u "' + $title + '"')
    $lines.Add('  Pop ' + $controls[$i])
    $lines.Add('  ${NSD_OnClick} ' + $controls[$i] + ' ModelRadioClicked')
    $lines.Add('  ${NSD_CreateLabel} 12u ' + (15 + 32*$i) + 'u 96% 16u "' + $model.details + '"')
    $lines.Add('  Pop $0')
}
$lines.Add('!macroend')
$lines.Add('!macro DeletePresetModels')
foreach ($model in $models) { $lines.Add('  Delete "$INSTDIR\models\' + $model.file + '"') }
$lines.Add('!macroend')
$lines.Add('Function SelectCatalogModel')
foreach ($model in $models) {
    $lines.Add('  ${If} $ModelPreset == "' + $model.id + '"')
    if ($model.id -eq 'canary') {
        $lines.Add('    StrCpy $ModelUrl "${DEFAULT_URL}"')
        $lines.Add('    StrCpy $ModelHash "${DEFAULT_SHA}"')
    } else {
        $lines.Add('    StrCpy $ModelUrl "' + $model.url + '"')
        $lines.Add('    StrCpy $ModelHash "' + $model.sha256 + '"')
    }
    $lines.Add('    StrCpy $ModelPath "models\' + $model.file + '"')
    $lines.Add('  ${EndIf}')
}
$lines.Add('FunctionEnd')
$lines.Add('Function ValidateCatalogModel')
$lines.Add('  StrCpy $ErrorText ""')
$lines.Add('  ${If} $ModelPreset == "whisper"')
$lines.Add('    Return')
$lines.Add('  ${EndIf}')
$lines.Add('  ${If} $ModelPreset != "canary"')
$lines.Add('  ${AndIf} $ModelPreset != "redux"')
$lines.Add('    StrCpy $ErrorText "Choose a valid model preset."')
$lines.Add('    Return')
$lines.Add('  ${EndIf}')
$lines.Add('  ${If} $SpeechLanguage == "auto"')
$lines.Add('  ${AndIf} $ModelPreset == "redux"')
$lines.Add('    Return')
$lines.Add('  ${EndIf}')
foreach ($language in $models[0].languages) {
    $lines.Add('  ${If} $SpeechLanguage == "' + $language + '"')
    $lines.Add('    Return')
    $lines.Add('  ${EndIf}')
}
$lines.Add('  StrCpy $ErrorText "This model does not support the selected speech language. Choose Whisper, or go back and select a supported language. Canary requires an explicit language."')
$lines.Add('FunctionEnd')
$lines.Add('Function DefaultModelForLanguage')
$lines.Add('  Call ValidateCatalogModel')
$lines.Add('  ${If} $ErrorText != ""')
$lines.Add('    StrCpy $ModelPreset "whisper"')
$lines.Add('    StrCpy $ErrorText ""')
$lines.Add('  ${EndIf}')
$lines.Add('FunctionEnd')
[IO.File]::WriteAllLines((Join-Path $OutputDir 'models.nsh'), $lines)
