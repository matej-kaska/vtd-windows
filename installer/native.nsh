; Use Windows crypto and text conversion instead of shipping crypto/encoding DLLs.
Var HashPath
Var HashValue
Var ConvertSource
Var ConvertTarget
Var ConvertFromUtf8
Var SetupMutex

!macro AcquireSetupLock PREFIX
Function ${PREFIX}AcquireSetupLock
  ; Keep the handle open until process exit, including while downloads run.
  System::Call 'kernel32::CreateMutexW(p 0, i 0, w "Local\${RUN_VALUE}.Setup") p .s ?e'
  Pop $0
  Pop $SetupMutex
  ${If} $SetupMutex == 0
  ${OrIf} $0 == 183
    MessageBox MB_OK|MB_ICONSTOP "VTD setup or uninstall is already running. Close it and retry." /SD IDOK
    SetErrorLevel 1
    Quit
  ${EndIf}
FunctionEnd
!macroend
!insertmacro AcquireSetupLock ""
!insertmacro AcquireSetupLock "un."

Function SHA256
  System::Store S
  StrCpy $HashValue ""
  StrCpy $0 0
  StrCpy $1 0
  StrCpy $2 -1
  StrCpy $3 0
  System::Call 'advapi32::CryptAcquireContextW(*p .r0, p 0, p 0, i 24, i 0xF0000000) i .r9'
  StrCmp $9 0 hash_cleanup
  System::Call 'advapi32::CryptCreateHash(p r0, i 0x800c, p 0, i 0, *p .r1) i .r9'
  StrCmp $9 0 hash_cleanup
  System::Call 'kernel32::CreateFileW(w "$HashPath", i 0x80000000, i 1, p 0, i 3, i 0x08000000, p 0) p .r2'
  StrCmp $2 -1 hash_cleanup
  System::Alloc 65536
  Pop $3
  StrCmp $3 0 hash_cleanup
hash_read:
  System::Call 'kernel32::ReadFile(p r2, p r3, i 65536, *i .r4, p 0) i .r9'
  StrCmp $9 0 hash_cleanup
  StrCmp $4 0 hash_finish
  System::Call 'advapi32::CryptHashData(p r1, p r3, i r4, i 0) i .r9'
  StrCmp $9 0 hash_cleanup hash_read
hash_finish:
  System::Call 'advapi32::CryptGetHashParam(p r1, i 2, p r3, *i 32, i 0) i .r9'
  StrCmp $9 0 hash_cleanup
  StrCpy $4 0
hash_hex:
  IntOp $5 $3 + $4
  System::Call '*$5(&i1 .r6)'
  IntFmt $6 "%02x" $6
  StrCpy $HashValue "$HashValue$6"
  IntOp $4 $4 + 1
  IntCmp $4 32 hash_cleanup hash_hex
hash_cleanup:
  System::Free $3
  ${If} $2 != -1
    System::Call 'kernel32::CloseHandle(p r2)'
  ${EndIf}
  ${If} $1 != 0
    System::Call 'advapi32::CryptDestroyHash(p r1)'
  ${EndIf}
  ${If} $0 != 0
    System::Call 'advapi32::CryptReleaseContext(p r0, i 0)'
  ${EndIf}
  System::Store L
FunctionEnd

; nsJSON's byte-file output uses the ANSI code page. Convert explicitly so
; non-ASCII Windows user names, microphone names and model paths round-trip.
Function ConvertConfig
  System::Store S
  StrCpy $1 -1
  StrCpy $2 0
  StrCpy $3 0
  StrCpy $8 0
  System::Call 'kernel32::CreateFileW(w "$ConvertSource", i 0x80000000, i 1, p 0, i 3, i 0, p 0) p .r0'
  StrCmp $0 -1 convert_cleanup
  System::Call 'kernel32::GetFileSize(p r0, p 0) i .r4'
  ${If} $4 <= 0
  ${OrIf} $4 > 1048576
    Goto convert_cleanup
  ${EndIf}
  System::Alloc $4
  Pop $2
  StrCmp $2 0 convert_cleanup
  System::Call 'kernel32::ReadFile(p r0, p r2, i r4, *i .r5, p 0) i .r9'
  StrCmp $9 0 convert_cleanup
  StrCmp $4 $5 0 convert_cleanup
  ${If} $ConvertFromUtf8 == 1
    System::Call 'kernel32::MultiByteToWideChar(i 65001, i 8, p r2, i r4, p 0, i 0) i .r6'
    StrCmp $6 0 convert_cleanup
    IntOp $7 $6 * 2
    System::Alloc $7
    Pop $3
    StrCmp $3 0 convert_cleanup
    System::Call 'kernel32::MultiByteToWideChar(i 65001, i 8, p r2, i r4, p r3, i r6) i .r9'
  ${Else}
    IntOp $4 $4 / 2
    System::Call 'kernel32::WideCharToMultiByte(i 65001, i 0x80, p r2, i r4, p 0, i 0, p 0, p 0) i .r7'
    StrCmp $7 0 convert_cleanup
    System::Alloc $7
    Pop $3
    StrCmp $3 0 convert_cleanup
    System::Call 'kernel32::WideCharToMultiByte(i 65001, i 0x80, p r2, i r4, p r3, i r7, p 0, p 0) i .r9'
  ${EndIf}
  StrCmp $9 0 convert_cleanup
  System::Call 'kernel32::CreateFileW(w "$ConvertTarget", i 0x40000000, i 0, p 0, i 2, i 0, p 0) p .r1'
  StrCmp $1 -1 convert_cleanup
  System::Call 'kernel32::WriteFile(p r1, p r3, i r7, *i .r5, p 0) i .r9'
  StrCmp $9 0 convert_cleanup
  StrCmp $5 $7 0 convert_cleanup
  StrCpy $8 1
convert_cleanup:
  ${If} $0 != -1
    System::Call 'kernel32::CloseHandle(p r0)'
  ${EndIf}
  ${If} $1 != -1
    System::Call 'kernel32::CloseHandle(p r1)'
  ${EndIf}
  System::Free $2
  System::Free $3
  ${If} $8 == 1
    ClearErrors
  ${Else}
    SetErrors
  ${EndIf}
  System::Store L
FunctionEnd

!macro StopOwnedInstance PREFIX
Function ${PREFIX}StopOwnedInstance
  ; Never stop another portable/installed copy just because it is named vtd.exe.
  System::Store S
  ClearErrors
  FindWindow $0 "VTDWindows"
  ${If} $0 != 0
    System::Call 'user32::GetWindowThreadProcessId(p r0, *i .r1)'
    System::Call 'kernel32::OpenProcess(i 0x101000, i 0, i r1) p .r2'
    ${If} $2 != 0
      System::Call 'kernel32::QueryFullProcessImageNameW(p r2, i 0, w .r3, *i ${NSIS_MAX_STRLEN}) i .r4'
      ${If} $4 != 0
      ${AndIf} $3 == "$INSTDIR\vtd.exe"
        System::Call 'user32::PostMessageW(p r0, i 0x10, p 0, p 0)'
        System::Call 'kernel32::WaitForSingleObject(p r2, i 15000) i .r4'
        ${If} $4 != 0
          SetErrors
        ${EndIf}
      ${EndIf}
      System::Call 'kernel32::CloseHandle(p r2)'
    ${Else}
      SetErrors
    ${EndIf}
  ${EndIf}
  System::Store L
FunctionEnd
!macroend
!insertmacro StopOwnedInstance ""
!insertmacro StopOwnedInstance "un."
