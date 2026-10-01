Unicode true
!include "LogicLib.nsh"
!include "nsDialogs.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"
!include "WinVer.nsh"
!addplugindir /x86-unicode "${TOOLS_DIR}\inetc\Plugins\x86-unicode"
!addplugindir /x86-unicode "${TOOLS_DIR}\nsjson\Plugins\x86-unicode"

!ifdef TEST_HARNESS
  !define PRODUCT_KEY "Software\VTD-Installer-Tests"
  !define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\VTD-Installer-Tests"
  !define RUN_VALUE "VTD Installer Test"
  !define SHORTCUT_FOLDER "VTD Installer Tests"
!else
  !define PRODUCT_KEY "Software\VTD"
  !define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\VTD"
  !define RUN_VALUE "VTD Windows"
  !define SHORTCUT_FOLDER "VTD"
!endif

Name "VTD"
Caption "VTD Setup"
OutFile "${OUTPUT_DIR}\VTD-Setup.exe"
InstallDir "$LOCALAPPDATA\Programs\VTD"
InstallDirRegKey HKCU "${PRODUCT_KEY}" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID ${COMPRESSION}
!if "${COMPRESSION}" == "lzma"
SetCompressorDictSize 1
!endif
SetDatablockOptimize on
XPStyle on
ManifestDPIAware true
Icon "..\assets\icon.ico"
UninstallIcon "..\assets\icon.ico"
ShowInstDetails show
ShowUninstDetails show
BrandingText "VTD - Offline voice dictation"
VIProductVersion "${VERSION_NUMBER}"
VIAddVersionKey /LANG=1033 "ProductName" "VTD Setup"
VIAddVersionKey /LANG=1033 "FileDescription" "VTD online installer"
VIAddVersionKey /LANG=1033 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "VTD contributors"

!include "${OUTPUT_DIR}\model-defines.nsh"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

Var HoldKey
Var ToggleKey
Var ReplayKey
Var SpeechLanguage
Var LangName
Var Mute
Var Autostart
Var Launch
Var Source
Var ModelPreset
Var ModelTwoControl
Var ModelThreeControl
Var CustomControl
Var ModelUrl
Var ModelFile
Var ModelHash
Var ModelPath
Var ModelStaged
Var OriginalModelPath
Var OriginalModelFile
Var OptionsFile
Var ErrorText
Var Page
Var HoldControl
Var ToggleControl
Var ReplayControl
Var LanguageControl
Var MuteControl
Var AutostartControl
Var LaunchControl
Var DefaultControl
Var UrlControl
Var FileControl
Var UrlEdit
Var FileEdit
Var HashEdit
Var RemoveData
Var RemoveDataControl
Var ConfigLoadedFrom

!include "native.nsh"
!include "${OUTPUT_DIR}\languages.nsh"
!include "${OUTPUT_DIR}\models.nsh"

Page directory "" DirectoryLeave
Page custom PreferencesPage PreferencesLeave
Page custom ModelPage ModelCatalogLeave
Page custom CustomModelPage ModelLeave
Page instfiles
UninstPage custom un.OptionsPage un.OptionsLeave
UninstPage instfiles
LoadLanguageFile "${NSISDIR}\Contrib\Language files\English.nlf"

!macro ReadOptional FIELD VARIABLE
  ClearErrors
  ReadINIStr $0 "$OptionsFile" "Settings" "${FIELD}"
  ${IfNot} ${Errors}
    StrCpy ${VARIABLE} $0
  ${EndIf}
!macroend

!macro ReadJson FIELD VARIABLE
  nsJSON::Get /exists "${FIELD}" /end
  Pop $0
  ${If} $0 == "yes"
    nsJSON::Get "${FIELD}" /end
    Pop ${VARIABLE}
  ${EndIf}
!macroend

Function .onInit
  SetErrorLevel 1
  Call AcquireSetupLock
  !ifdef TEST_HARNESS
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "Stage" "onInit"
  !endif
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "VTD requires x64 Windows." /SD IDOK
    SetErrorLevel 1
    Quit
  ${EndIf}
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_OK|MB_ICONSTOP "VTD requires Windows 10 or 11." /SD IDOK
    SetErrorLevel 1
    Quit
  ${EndIf}
  SetRegView 64
  SetShellVarContext current
  ${GetOptions} $CMDLINE "/D=" $0
  ${If} ${Errors}
    ReadRegStr $0 HKCU "${PRODUCT_KEY}" "InstallDir"
    ${If} $0 != ""
      StrCpy $INSTDIR $0
    ${EndIf}
  ${EndIf}
  ${DisableX64FSRedirection}
  IfFileExists "$SYSDIR\tar.exe" +4
    MessageBox MB_OK|MB_ICONSTOP "This version of Windows is missing tar.exe. Install current Windows updates and retry." /SD IDOK
    SetErrorLevel 1
    Quit
  InitPluginsDir
  StrCpy $ConfigLoadedFrom ""
  ${GetParameters} $0
  ${GetOptions} $0 "/OPTIONS=" $OptionsFile
  !ifdef TEST_HARNESS
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "Stage" "initialized"
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "Options" "$OptionsFile"
  !endif
FunctionEnd

Function DefaultPreferences
  StrCpy $HoldKey 119
  StrCpy $ToggleKey 120
  StrCpy $ReplayKey 121
  StrCpy $Mute 0
  StrCpy $Autostart 1
  StrCpy $Launch 1
  StrCpy $Source "default"
  StrCpy $ModelPreset "canary"
  StrCpy $ModelUrl "${DEFAULT_URL}"
  StrCpy $ModelHash ""
  StrCpy $ModelFile ""
  StrCpy $OriginalModelPath ""
  StrCpy $OriginalModelFile ""
  System::Call 'kernel32::GetUserDefaultUILanguage() i .r0'
  System::Call 'kernel32::LCIDToLocaleName(i r0, w .r1, i ${NSIS_MAX_STRLEN}, i 0) i .r2'
  StrCpy $SpeechLanguage ""
  StrCpy $0 0
locale_loop:
  StrCpy $2 $1 1 $0
  StrCmp $2 "" locale_done
  StrCmp $2 "-" locale_done
  StrCpy $SpeechLanguage "$SpeechLanguage$2"
  IntOp $0 $0 + 1
  Goto locale_loop
locale_done:
  ${Switch} $SpeechLanguage
    ${Case} "nb"
      StrCpy $SpeechLanguage "no"
      ${Break}
    ${Case} "fil"
      StrCpy $SpeechLanguage "tl"
      ${Break}
    ${Case} "jv"
      StrCpy $SpeechLanguage "jw"
      ${Break}
    ${Case} "iw"
      StrCpy $SpeechLanguage "he"
      ${Break}
  ${EndSwitch}
  Call LanguageName
  Call LanguageCode
  Call DefaultModelForLanguage
  IfSilent 0 +2
    StrCpy $Launch 0
FunctionEnd

Function DirectoryLeave
  !ifdef TEST_HARNESS
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "Stage" "DirectoryLeave"
  !endif
  ; The NSIS instruction requires an existing path; a fresh install has none.
  System::Call 'kernel32::GetFullPathNameW(w "$INSTDIR", i ${NSIS_MAX_STRLEN}, w .r1, p 0) i .r2'
  ${If} $2 == 0
  ${OrIf} $2 >= ${NSIS_MAX_STRLEN}
    MessageBox MB_OK|MB_ICONSTOP "Choose a shorter, valid installation path." /SD IDOK
    Abort
  ${EndIf}
  StrCpy $INSTDIR $1
  ${GetRoot} "$INSTDIR" $0
  !ifdef TEST_HARNESS
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "Directory" "$INSTDIR"
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "Root" "$0"
  !endif
  ${If} $INSTDIR == "$0\"
  ${OrIf} $INSTDIR == $0
  ${OrIf} $INSTDIR == $LOCALAPPDATA
  ${OrIf} $INSTDIR == $PROFILE
  ${OrIf} $INSTDIR == $WINDIR
    MessageBox MB_OK|MB_ICONSTOP "Choose a dedicated VTD folder." /SD IDOK
    Abort
  ${EndIf}
  ${If} $ConfigLoadedFrom == $INSTDIR
    Return
  ${EndIf}
  Call DefaultPreferences
  nsJSON::Set /value "{}"
  !ifdef TEST_HARNESS
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "Stage" "config-created"
  !endif
  IfFileExists "$INSTDIR\vtd.json" 0 config_loaded
  StrCpy $ConvertSource "$INSTDIR\vtd.json"
  StrCpy $ConvertTarget "$PLUGINSDIR\existing.utf16"
  StrCpy $ConvertFromUtf8 1
  Call ConvertConfig
  IfErrors config_invalid
  nsJSON::Set /file /unicode "$PLUGINSDIR\existing.utf16"
  IfErrors config_invalid
  !insertmacro ReadJson "trigger_key" $HoldKey
  !insertmacro ReadJson "toggle_key" $ToggleKey
  !insertmacro ReadJson "replay_key" $ReplayKey
  !insertmacro ReadJson "language" $SpeechLanguage
  !insertmacro ReadJson "mute_output" $Mute
  ${If} $Mute == "true"
    StrCpy $Mute 1
  ${Else}
    StrCpy $Mute 0
  ${EndIf}
  !insertmacro ReadJson "model" $ModelFile
  StrCpy $OriginalModelPath $ModelFile
  ${If} $ModelFile != ""
    ${GetRoot} "$ModelFile" $0
    ${If} $0 == ""
      GetFullPathName $ModelFile "$INSTDIR\$ModelFile"
    ${EndIf}
    IfFileExists "$ModelFile" 0 +2
      StrCpy $Source "file"
    StrCpy $OriginalModelFile $ModelFile
  ${EndIf}
  StrCpy $Autostart 0
  ReadRegStr $0 HKCU "${RUN_KEY}" "${RUN_VALUE}"
  ${If} $0 == '$\"$INSTDIR\vtd.exe$\" run'
    StrCpy $Autostart 1
  ${EndIf}
config_loaded:
  !ifdef TEST_HARNESS
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "Stage" "config-loaded"
  !endif
  StrCpy $ConfigLoadedFrom $INSTDIR
  ${If} $OptionsFile != ""
    IfFileExists "$OptionsFile" +4
      MessageBox MB_OK|MB_ICONSTOP "The options file does not exist." /SD IDOK
      SetErrorLevel 1
      Abort
    !insertmacro ReadOptional "HoldKey" $HoldKey
    !insertmacro ReadOptional "ToggleKey" $ToggleKey
    !insertmacro ReadOptional "ReplayKey" $ReplayKey
    !insertmacro ReadOptional "Language" $SpeechLanguage
    !insertmacro ReadOptional "Mute" $Mute
    !insertmacro ReadOptional "Autostart" $Autostart
    !insertmacro ReadOptional "Launch" $Launch
    !insertmacro ReadOptional "ModelSource" $Source
    !insertmacro ReadOptional "ModelPreset" $ModelPreset
    !insertmacro ReadOptional "ModelUrl" $ModelUrl
    !insertmacro ReadOptional "ModelFile" $ModelFile
    !insertmacro ReadOptional "ModelHash" $ModelHash
  ${EndIf}
  !ifdef TEST_HARNESS
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "Stage" "options-loaded"
  !endif
  Return
config_invalid:
  MessageBox MB_OK|MB_ICONSTOP "Cannot read the existing vtd.json. It has not been changed." /SD IDOK
  SetErrorLevel 1
  Abort
FunctionEnd

!macro KeyCombo LABEL TOP CONTROL KEY
  ${NSD_CreateLabel} 0 ${TOP}u 54% 12u "${LABEL}"
  Pop $0
  ${NSD_CreateHotKey} 56% ${TOP}u 44% 14u ""
  Pop ${CONTROL}
  ${NSD_HK_SetRules} ${CONTROL} 0 0
  ${NSD_HK_SetHotKey} ${CONTROL} ${KEY}
!macroend

Function PreferencesPage
  ${NSD_InitCommonControlsEx} 0x0040
  nsDialogs::Create 1018
  Pop $Page
  !insertmacro KeyCombo "Hold to record:" 0 $HoldControl $HoldKey
  !insertmacro KeyCombo "Start / stop recording:" 17 $ToggleControl $ToggleKey
  !insertmacro KeyCombo "Insert last transcript:" 34 $ReplayControl $ReplayKey
  ${NSD_CreateLabel} 0 54u 40% 12u "Speech language:"
  Pop $0
  ${NSD_CreateDropList} 42% 52u 58% 100u ""
  Pop $LanguageControl
  !insertmacro LanguageItems $LanguageControl
  Call LanguageName
  ${NSD_CB_SelectString} $LanguageControl $LangName
  ${NSD_CreateCheckbox} 0 67u 100% 12u "Mute playback while recording"
  Pop $MuteControl
  ${NSD_SetState} $MuteControl $Mute
  ${NSD_CreateCheckbox} 0 80u 100% 12u "Start when I sign in to Windows"
  Pop $AutostartControl
  ${NSD_SetState} $AutostartControl $Autostart
  ${NSD_CreateCheckbox} 0 93u 100% 12u "Start VTD after installation"
  Pop $LaunchControl
  ${NSD_SetState} $LaunchControl $Launch
  ${NSD_CreateLabel} 0 108u 100% 24u "Click a field, then press your key without Fn. You can also use Ctrl, Shift or Alt."
  Pop $0
  nsDialogs::Show
FunctionEnd

!macro ValidateKey KEY
  IntOp $0 ${KEY} & 255
  IntOp $1 ${KEY} & 0xFFFFF800
  ${If} $1 != 0
  ${OrIf} $0 < 32
  ${OrIf} $0 > 254
  ${OrIf} $0 == 91
  ${OrIf} $0 == 92
  ${OrIf} $0 == 229
  ${OrIf} $0 == 231
    StrCpy $ErrorText "Choose a key, optionally with Ctrl, Shift or Alt."
  ${EndIf}
  ${If} $0 >= 160
  ${AndIf} $0 <= 165
    StrCpy $ErrorText "Choose a key, optionally with Ctrl, Shift or Alt."
  ${EndIf}
!macroend

Function ValidatePreferences
  StrCpy $ErrorText ""
  !insertmacro ValidateKey $HoldKey
  !insertmacro ValidateKey $ToggleKey
  !insertmacro ValidateKey $ReplayKey
  ${If} $HoldKey == $ToggleKey
  ${OrIf} $HoldKey == $ReplayKey
  ${OrIf} $ToggleKey == $ReplayKey
    StrCpy $ErrorText "Each action must use a different key."
  ${EndIf}
  StrCpy $0 $SpeechLanguage
  Call LanguageName
  Call LanguageCode
  ${If} $0 != $SpeechLanguage
    StrCpy $ErrorText "Choose a supported speech language."
  ${EndIf}
  ${If} $Mute != 0
  ${AndIf} $Mute != 1
    StrCpy $ErrorText "Mute must be 0 or 1."
  ${EndIf}
  ${If} $Autostart != 0
  ${AndIf} $Autostart != 1
    StrCpy $ErrorText "Autostart must be 0 or 1."
  ${EndIf}
  ${If} $Launch != 0
  ${AndIf} $Launch != 1
    StrCpy $ErrorText "Launch must be 0 or 1."
  ${EndIf}
FunctionEnd

Function PreferencesLeave
  ${NSD_HK_GetHotKey} $HoldControl $HoldKey
  ${NSD_HK_GetHotKey} $ToggleControl $ToggleKey
  ${NSD_HK_GetHotKey} $ReplayControl $ReplayKey
  IntOp $HoldKey $HoldKey & 2047
  IntOp $ToggleKey $ToggleKey & 2047
  IntOp $ReplayKey $ReplayKey & 2047
  ${NSD_GetText} $LanguageControl $LangName
  Call LanguageCode
  ${NSD_GetState} $MuteControl $Mute
  ${NSD_GetState} $AutostartControl $Autostart
  ${NSD_GetState} $LaunchControl $Launch
  Call ValidatePreferences
  ${If} $ErrorText != ""
    MessageBox MB_OK|MB_ICONEXCLAMATION "$ErrorText"
    Abort
  ${EndIf}
FunctionEnd

Function ModelPage
  nsDialogs::Create 1018
  Pop $Page
  !insertmacro ModelChoices
  ${NSD_CreateLabel} 0 99u 100% 27u "${MODEL_BENCHMARK}"
  Pop $0
  ${NSD_CreateRadioButton} 0 132u 100% 12u "Keep an existing model or use a custom download"
  Pop $CustomControl
  ${NSD_OnClick} $CustomControl ModelRadioClicked
  ${If} $Source != "default"
    ${NSD_Check} $CustomControl
  ${ElseIf} $ModelPreset == "parakeet"
    ${NSD_Check} $ModelTwoControl
  ${ElseIf} $ModelPreset == "whisper"
    ${NSD_Check} $ModelThreeControl
  ${Else}
    ${NSD_Check} $DefaultControl
  ${EndIf}
  nsDialogs::Show
FunctionEnd

Function ModelRadioClicked
  Pop $0
  ${NSD_Uncheck} $DefaultControl
  ${NSD_Uncheck} $ModelTwoControl
  ${NSD_Uncheck} $ModelThreeControl
  ${NSD_Uncheck} $CustomControl
  ${NSD_Check} $0
FunctionEnd

Function ModelCatalogLeave
  ${NSD_GetState} $CustomControl $0
  ${If} $0 == 1
    ${If} $Source == "default"
      StrCpy $Source "url"
      ${If} $ModelFile != ""
        StrCpy $Source "file"
      ${EndIf}
    ${EndIf}
    Return
  ${EndIf}
  StrCpy $Source "default"
  StrCpy $ModelPreset "canary"
  ${NSD_GetState} $ModelTwoControl $0
  ${If} $0 == 1
    StrCpy $ModelPreset "parakeet"
  ${EndIf}
  ${NSD_GetState} $ModelThreeControl $0
  ${If} $0 == 1
    StrCpy $ModelPreset "whisper"
  ${EndIf}
  Call ValidateModel
  ${If} $ErrorText != ""
    MessageBox MB_OK|MB_ICONEXCLAMATION "$ErrorText"
    Abort
  ${EndIf}
FunctionEnd

Function CustomModelPage
  ${If} $Source == "default"
    Abort
  ${EndIf}
  nsDialogs::Create 1018
  Pop $Page
  ${NSD_CreateRadioButton} 0 4u 100% 12u "Download a model from a custom HTTPS link"
  Pop $UrlControl
  ${NSD_CreateText} 12u 19u 95% 13u "$ModelUrl"
  Pop $UrlEdit
  ${NSD_CreateLabel} 12u 38u 40% 12u "SHA-256 (optional):"
  Pop $0
  ${NSD_CreateText} 44% 36u 56% 13u "$ModelHash"
  Pop $HashEdit
  ${NSD_CreateRadioButton} 0 61u 100% 12u "Use an existing model file (keep its location)"
  Pop $FileControl
  ${NSD_CreateText} 12u 77u 72% 13u "$ModelFile"
  Pop $FileEdit
  ${NSD_CreateButton} 79% 76u 21% 15u "Browse..."
  Pop $0
  ${NSD_OnClick} $0 BrowseModel
  ${NSD_CreateLabel} 0 102u 100% 30u "Whisper GGML (.bin), Canary or Parakeet GGUF (.gguf). Preset models are verified with SHA-256."
  Pop $0
  ${If} $Source == "file"
    ${NSD_Check} $FileControl
  ${Else}
    ${NSD_Check} $UrlControl
  ${EndIf}
  nsDialogs::Show
FunctionEnd

Function BrowseModel
  Pop $0
  nsDialogs::SelectFileDialog open "$ModelFile" "Speech models (*.bin;*.gguf)|*.bin;*.gguf|All files (*.*)|*.*"
  Pop $0
  ${If} $0 != ""
    ${NSD_SetText} $FileEdit $0
    ${NSD_Uncheck} $UrlControl
    ${NSD_Check} $FileControl
  ${EndIf}
FunctionEnd

Function ModelLeave
  ${If} $Source == "default"
    Return
  ${EndIf}
  StrCpy $Source "url"
  ${NSD_GetState} $UrlControl $0
  ${If} $0 == 1
    StrCpy $Source "url"
  ${EndIf}
  ${NSD_GetState} $FileControl $0
  ${If} $0 == 1
    StrCpy $Source "file"
  ${EndIf}
  ${NSD_GetText} $UrlEdit $ModelUrl
  ${NSD_GetText} $HashEdit $ModelHash
  ${NSD_GetText} $FileEdit $ModelFile
  Call ValidateModel
  ${If} $ErrorText != ""
    MessageBox MB_OK|MB_ICONEXCLAMATION "$ErrorText"
    Abort
  ${EndIf}
FunctionEnd

Function ValidateModel
  StrCpy $ErrorText ""
  ${If} $Source == "default"
    Call ValidateCatalogModel
    Return
  ${EndIf}
  ${If} $Source == "file"
    IfFileExists "$ModelFile" +2
      StrCpy $ErrorText "Choose an existing model file."
    Return
  ${EndIf}
  ${If} $Source != "url"
    StrCpy $ErrorText "Choose a model source."
    Return
  ${EndIf}
  StrCpy $0 $ModelUrl 8
  ${If} $0 != "https://"
    StrCpy $ErrorText "The model link must start with https://."
    !ifdef TEST_HARNESS
      StrCpy $0 $ModelUrl 17
      ${If} $0 == "http://127.0.0.1:"
        StrCpy $ErrorText ""
      ${EndIf}
    !endif
  ${EndIf}
  ${If} $ModelHash != ""
    StrLen $0 $ModelHash
    ${If} $0 != 64
      StrCpy $ErrorText "SHA-256 must contain 64 hexadecimal characters, or be empty."
    ${EndIf}
    System::Call 'shlwapi::StrSpnW(w "$ModelHash", w "0123456789abcdefABCDEF") i .r0'
    ${If} $0 != 64
      StrCpy $ErrorText "SHA-256 must contain 64 hexadecimal characters, or be empty."
    ${EndIf}
  ${EndIf}
FunctionEnd

!macro RuntimeFiles ACTION
  !insertmacro ${ACTION} "vtd.exe"
  !insertmacro ${ACTION} "vtd-engine.exe"
  !insertmacro ${ACTION} "vtd-transcribe.exe"
  !insertmacro ${ACTION} "msvcp140.dll"
  !insertmacro ${ACTION} "vcruntime140.dll"
  !insertmacro ${ACTION} "vcruntime140_1.dll"
  !insertmacro ${ACTION} "LICENSE"
  !insertmacro ${ACTION} "THIRD_PARTY_LICENSES.txt"
  !insertmacro ${ACTION} "INSTALLER_LICENSES.txt"
!macroend

!macro CheckRuntime NAME
  IfFileExists "$PLUGINSDIR\app\${NAME}" +3
    StrCpy $ErrorText "The application download is incomplete."
    Goto install_failed
!macroend

!macro BackupFile NAME
  IfFileExists "$INSTDIR\${NAME}" 0 +2
    CopyFiles /SILENT "$INSTDIR\${NAME}" "$PLUGINSDIR\backup\${NAME}"
  IfErrors backup_failed
!macroend

!macro CommitFile NAME
  CopyFiles /SILENT "$PLUGINSDIR\app\${NAME}" "$INSTDIR\${NAME}"
  IfErrors commit_failed
!macroend

!macro RollbackFile NAME
  IfFileExists "$PLUGINSDIR\backup\${NAME}" 0 +3
    CopyFiles /SILENT "$PLUGINSDIR\backup\${NAME}" "$INSTDIR\${NAME}"
    Goto +2
  Delete "$INSTDIR\${NAME}"
!macroend

Section "Install"
  AddSize ${INSTALLED_KIB}
  Call DirectoryLeave
  Call ValidatePreferences
  StrCmp $ErrorText "" 0 install_failed
  Call ValidateModel
  StrCmp $ErrorText "" 0 install_failed
  DetailPrint "Preparing VTD ${APP_VERSION}..."
  StrCpy $HashPath "$EXEDIR\vtd-runtime-x64.zip"
  IfFileExists "$HashPath" 0 download_app
  Call SHA256
  StrCmp $HashValue "${PAYLOAD_SHA256}" local_app
  StrCpy $ErrorText "The adjacent application ZIP does not match this installer."
  Goto install_failed
local_app:
  ClearErrors
  CopyFiles /SILENT "$HashPath" "$PLUGINSDIR\runtime.zip"
  ${If} ${Errors}
    StrCpy $ErrorText "Cannot copy the adjacent application ZIP. Check free space and retry."
    Goto install_failed
  ${EndIf}
  Goto extract_app
download_app:
  inetc::get /CONNECTTIMEOUT 30 /RECEIVETIMEOUT 60 /RESUME "Connection interrupted. Retry the application download?" /NOCOOKIES "${PAYLOAD_URL}" "$PLUGINSDIR\runtime.zip" /END
  Pop $0
  ${If} $0 != "OK"
    StrCpy $ErrorText "Application download failed: $0"
    Goto install_failed
  ${EndIf}
  StrCpy $HashPath "$PLUGINSDIR\runtime.zip"
  Call SHA256
  ${If} $HashValue != "${PAYLOAD_SHA256}"
    StrCpy $ErrorText "Application SHA-256 mismatch. No application files were installed."
    Goto install_failed
  ${EndIf}
extract_app:
  CreateDirectory "$PLUGINSDIR\app"
  nsExec::ExecToStack '"$SYSDIR\tar.exe" -xf "$PLUGINSDIR\runtime.zip" -C "$PLUGINSDIR\app"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    StrCpy $ErrorText "Cannot extract the application: $1"
    Goto install_failed
  ${EndIf}
  !insertmacro RuntimeFiles CheckRuntime
  StrCpy $ModelStaged ""
  ${If} $Source == "file"
    GetFullPathName $ModelPath "$ModelFile"
    StrCpy $ModelStaged $ModelPath
    ${If} $ModelFile == $OriginalModelFile
      StrCpy $ModelPath $OriginalModelPath
    ${EndIf}
  ${Else}
    ${If} $Source == "default"
      Call SelectCatalogModel
    ${Else}
      StrCpy $ModelPath "models\custom-model.bin"
    ${EndIf}
    ${If} $ModelHash != ""
      StrCpy $HashPath "$INSTDIR\$ModelPath"
      IfFileExists "$HashPath" 0 model_download
      DetailPrint "Checking the existing model..."
      Call SHA256
      ${If} $HashValue == $ModelHash
        StrCpy $ModelStaged "$HashPath"
        Goto model_ready
      ${EndIf}
    ${EndIf}
model_download:
    DetailPrint "Downloading the speech model..."
    inetc::get /CONNECTTIMEOUT 30 /RECEIVETIMEOUT 60 /RESUME "Connection interrupted. Retry and resume the model download?" /NOCOOKIES "$ModelUrl" "$PLUGINSDIR\model.part" /END
    Pop $0
    ${If} $0 != "OK"
      StrCpy $ErrorText "Model download failed: $0"
      Goto install_failed
    ${EndIf}
    StrCpy $ModelStaged "$PLUGINSDIR\model.part"
    ${If} $ModelHash != ""
      DetailPrint "Verifying the model SHA-256..."
      StrCpy $HashPath $ModelStaged
      Call SHA256
      ${If} $HashValue != $ModelHash
        StrCpy $ErrorText "Model SHA-256 mismatch. The existing installation has not been changed."
        Goto install_failed
      ${EndIf}
    ${EndIf}
  ${EndIf}
model_ready:
  ClearErrors
  FileOpen $0 "$ModelStaged" r
  IfErrors invalid_model
  FileReadByte $0 $1
  FileReadByte $0 $2
  FileReadByte $0 $3
  FileReadByte $0 $4
  FileClose $0
  ${If} $1 == 108
  ${AndIf} $2 == 109
  ${AndIf} $3 == 103
  ${AndIf} $4 == 103
    Goto valid_model_header
  ${EndIf}
  ${If} $1 != 71
  ${OrIf} $2 != 71
  ${OrIf} $3 != 85
  ${OrIf} $4 != 70
    Goto invalid_model
  ${EndIf}
valid_model_header:
  nsJSON::Set "trigger_key" /value "$HoldKey"
  nsJSON::Set "toggle_key" /value "$ToggleKey"
  nsJSON::Set "replay_key" /value "$ReplayKey"
  nsJSON::Quote /always "$SpeechLanguage"
  Pop $0
  nsJSON::Set "language" /value "$0"
  nsJSON::Quote /always "$ModelPath"
  Pop $0
  nsJSON::Set "model" /value "$0"
  ${If} $Mute == 1
    nsJSON::Set "mute_output" /value "true"
  ${Else}
    nsJSON::Set "mute_output" /value "false"
  ${EndIf}
  nsJSON::Serialize /format /file /unicode "$PLUGINSDIR\config.utf16"
  IfErrors config_write_failed
  StrCpy $ConvertSource "$PLUGINSDIR\config.utf16"
  StrCpy $ConvertTarget "$PLUGINSDIR\app\vtd.json"
  StrCpy $ConvertFromUtf8 0
  Call ConvertConfig
  IfErrors config_write_failed
  nsExec::ExecToStack '"$PLUGINSDIR\app\vtd.exe" check-config "$PLUGINSDIR\app\vtd.json"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    StrCpy $ErrorText "Configuration validation failed: $1"
    Goto install_failed
  ${EndIf}
  Call StopOwnedInstance
  ${If} ${Errors}
    StrCpy $ErrorText "Close the running VTD application and retry."
    Goto install_failed
  ${EndIf}
  CreateDirectory "$PLUGINSDIR\backup"
  CreateDirectory "$INSTDIR"
  ClearErrors
  !insertmacro RuntimeFiles BackupFile
  !insertmacro BackupFile "vtd.json"
  !insertmacro BackupFile "Uninstall.exe"
  WriteUninstaller "$PLUGINSDIR\app\Uninstall.exe"
  IfErrors backup_failed
  !insertmacro RuntimeFiles CommitFile
  !insertmacro CommitFile "vtd.json"
  !insertmacro CommitFile "Uninstall.exe"
  ${If} $ModelStaged == "$PLUGINSDIR\model.part"
    CreateDirectory "$INSTDIR\models"
    ; Replace only after a complete verified download; leave other models alone.
    CopyFiles /SILENT "$ModelStaged" "$INSTDIR\$ModelPath.part"
    IfErrors commit_failed
    System::Call 'kernel32::MoveFileExW(w "$INSTDIR\$ModelPath.part", w "$INSTDIR\$ModelPath", i 9) i .r0'
    StrCmp $0 0 commit_failed
  ${EndIf}
  ClearErrors
  WriteRegStr HKCU "${PRODUCT_KEY}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "VTD"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "VTD contributors"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\vtd.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  CreateDirectory "$SMPROGRAMS\${SHORTCUT_FOLDER}"
  CreateShortCut "$SMPROGRAMS\${SHORTCUT_FOLDER}\VTD.lnk" "$INSTDIR\vtd.exe" "run"
  IfErrors registration_failed
  ${If} $Autostart == 1
    WriteRegStr HKCU "${RUN_KEY}" "${RUN_VALUE}" '"$INSTDIR\vtd.exe" run'
    IfErrors registration_failed
  ${Else}
    ReadRegStr $0 HKCU "${RUN_KEY}" "${RUN_VALUE}"
    ${If} $0 == '"$INSTDIR\vtd.exe" run'
      DeleteRegValue HKCU "${RUN_KEY}" "${RUN_VALUE}"
      IfErrors registration_failed
    ${EndIf}
  ${EndIf}
  DetailPrint "VTD is installed. Settings are available from its tray icon."
  ${If} $Launch == 1
    Exec '"$INSTDIR\vtd.exe" run'
  ${EndIf}
  SetErrorLevel 0
  Goto install_done
registration_failed:
  StrCpy $ErrorText "VTD files are installed, but Windows shortcuts or startup registration failed. Retry setup from the same folder."
  Goto install_failed
invalid_model:
  StrCpy $ErrorText "This file is not a compatible GGML or GGUF speech model."
  Goto install_failed
config_write_failed:
  StrCpy $ErrorText "Cannot prepare the configuration. No application files were changed."
  Goto install_failed
commit_failed:
  StrCpy $ErrorText "Cannot write the installation. Check free space and close VTD before retrying."
  !insertmacro RuntimeFiles RollbackFile
  !insertmacro RollbackFile "vtd.json"
  !insertmacro RollbackFile "Uninstall.exe"
  Delete "$INSTDIR\$ModelPath.part"
  Goto install_failed
backup_failed:
  StrCpy $ErrorText "Cannot back up the existing installation. Its files have not been changed."
install_failed:
  !ifdef TEST_HARNESS
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "Error" "$ErrorText"
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "InstallDir" "$INSTDIR"
  !endif
  DetailPrint "$ErrorText"
  MessageBox MB_OK|MB_ICONSTOP "$ErrorText" /SD IDOK
  SetErrorLevel 1
  Abort
install_done:
  !ifdef TEST_HARNESS
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "Error" ""
    WriteINIStr "$EXEDIR\test-status.ini" "Result" "InstallDir" "$INSTDIR"
  !endif
SectionEnd

Function un.onInit
  Call un.AcquireSetupLock
  SetRegView 64
  SetShellVarContext current
  StrCpy $RemoveData 0
  ${GetParameters} $0
  ${GetOptions} $0 "/PURGE" $1
  ${IfNot} ${Errors}
    StrCpy $RemoveData 1
  ${EndIf}
FunctionEnd

Function un.OptionsPage
  nsDialogs::Create 1018
  Pop $Page
  ${NSD_CreateLabel} 0 4u 100% 35u "Remove VTD from this computer? Your model files and settings will be kept unless you select the option below."
  Pop $0
  ${NSD_CreateCheckbox} 0 48u 100% 20u "Also remove downloaded models and settings"
  Pop $RemoveDataControl
  nsDialogs::Show
FunctionEnd

Function un.OptionsLeave
  ${NSD_GetState} $RemoveDataControl $RemoveData
FunctionEnd

!macro DeleteRuntime NAME
  Delete "$INSTDIR\${NAME}"
!macroend

Section "Uninstall"
  Call un.StopOwnedInstance
  ${If} ${Errors}
    MessageBox MB_OK|MB_ICONSTOP "Close VTD and retry uninstalling." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  !insertmacro RuntimeFiles DeleteRuntime
  ${If} ${Errors}
    MessageBox MB_OK|MB_ICONSTOP "Some VTD files could not be removed. Close applications using this folder and run uninstall again." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ReadRegStr $0 HKCU "${RUN_KEY}" "${RUN_VALUE}"
  ${If} $0 == '"$INSTDIR\vtd.exe" run'
    DeleteRegValue HKCU "${RUN_KEY}" "${RUN_VALUE}"
  ${EndIf}
  ReadRegStr $0 HKCU "${UNINSTALL_KEY}" "InstallLocation"
  ${If} $0 == $INSTDIR
    DeleteRegKey HKCU "${UNINSTALL_KEY}"
    DeleteRegKey HKCU "${PRODUCT_KEY}"
    Delete "$SMPROGRAMS\${SHORTCUT_FOLDER}\VTD.lnk"
    RMDir "$SMPROGRAMS\${SHORTCUT_FOLDER}"
  ${EndIf}
  ${If} $RemoveData == 1
    Delete "$INSTDIR\vtd.json"
    !insertmacro DeletePresetModels
    Delete "$INSTDIR\models\custom-model.bin"
    RMDir "$INSTDIR\models"
  ${EndIf}
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  SetErrorLevel 0
SectionEnd
