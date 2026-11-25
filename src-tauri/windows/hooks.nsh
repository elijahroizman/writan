; The Windows installer and uninstaller both stop if they cannot close Writan.
; Tauri's template asks Restart Manager to shut down whatever has
; $INSTDIR\${MAINBINARYNAME}.exe open, and aborts when that call fails:
; "Failed to kill Writan. Please close it first then try again".
;
; An update starts this installer and then exits the running app with
; process::exit. Restart Manager cannot shut down a process that is already
; exiting, so the update aborts. The same check aborts an uninstall while
; Writan is open. See tauri-apps/tauri#16277 and #12309.
;
; Wait until that file is free. If it is still in use, end the process that
; has it open and wait again. A normal install, with Writan open and no
; update in progress, still asks before closing it.

!macro StopInstalledWorkspace
  ; The path is passed through the environment so a quote or space in the
  ; user name cannot break the PowerShell command.
  System::Call 'kernel32::SetEnvironmentVariable(t "WORKSPACE_INSTALL_EXE", t "$INSTDIR\${MAINBINARYNAME}.exe")'
  System::Call 'kernel32::SetEnvironmentVariable(t "WORKSPACE_LEGACY_EXE", t "$INSTDIR\Workspace.exe")'
  nsExec::Exec '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -WindowStyle Hidden -Command "Get-CimInstance Win32_Process | Where-Object { $$_.ExecutablePath -eq $$env:WORKSPACE_INSTALL_EXE -or $$_.ExecutablePath -eq $$env:WORKSPACE_LEGACY_EXE } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force -ErrorAction SilentlyContinue }"'
  Pop $1
  System::Call 'kernel32::SetEnvironmentVariable(t "WORKSPACE_INSTALL_EXE", p0)'
  System::Call 'kernel32::SetEnvironmentVariable(t "WORKSPACE_LEGACY_EXE", p0)'
!macroend

; mode "grace": an update. Give the app a moment to finish exiting, then end it.
; mode "now": an uninstall. End it on the first check.
!macro CloseInstalledWorkspace unique mode message
  Push $0
  Push $1
  Push $2
  Push $3
  Push $R7
  Push $R8
  Push $R9
  StrCpy $R9 0
  StrCpy $R7 0
  ws_wait_${unique}:
    StrCpy $0 0
    !insertmacro RestartManager_StartSession $R8
    ${If} $R8 == ""
      ${If} $R7 = 0
        DetailPrint "${message}"
        StrCpy $R7 1
        !insertmacro StopInstalledWorkspace
      ${EndIf}
      Goto ws_done_${unique}
    ${EndIf}
    ${If} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
      !insertmacro RestartManager_RegisterFile $R8 "$INSTDIR\${MAINBINARYNAME}.exe"
    ${EndIf}
    ${If} ${FileExists} "$INSTDIR\Workspace.exe"
      !insertmacro RestartManager_RegisterFile $R8 "$INSTDIR\Workspace.exe"
    ${EndIf}
    ${If} $0 = 0
      ; ERROR_MORE_DATA means some process still has the installed exe open.
      System::Call 'RSTRTMGR::RmGetList(p R8, *i .r1, *i .r2, p 0, *i .r3) i .r0'
    ${EndIf}
    !insertmacro RestartManager_EndSession $R8
    ${If} $0 != ${ERROR_MORE_DATA}
      Goto ws_done_${unique}
    ${EndIf}
    ${If} $R9 >= 80
      Goto ws_done_${unique}
    ${EndIf}
    ${If} $R9 = 0
      DetailPrint "${message}"
    ${EndIf}
    IntOp $0 $R9 % 8
    ${If} $0 = 0
      !if "${mode}" == "grace"
        ${If} $R9 > 0
          !insertmacro StopInstalledWorkspace
        ${EndIf}
      !else
        !insertmacro StopInstalledWorkspace
      !endif
    ${EndIf}
    Sleep 250
    IntOp $R9 $R9 + 1
    Goto ws_wait_${unique}
  ws_done_${unique}:
  Pop $R9
  Pop $R8
  Pop $R7
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; 0.9.x used a product-name registry key. Keep that install's directory,
  ; including custom locations. Reset the output path too: Tauri set it before
  ; calling this hook. Do not run the old uninstaller or touch WebView/app data.
  ReadRegStr $0 HKCU "${MANUKEY}\Workspace" ""
  ${If} $0 != ""
  ${AndIf} ${FileExists} "$0\Workspace.exe"
    StrCpy $INSTDIR $0
    SetOutPath $INSTDIR
  ${EndIf}
  ; Passive and silent installs have no "close it?" question. The updater
  ; passes /P and /UPDATE. A manual install still asks.
  ${If} $UpdateMode = 1
  ${OrIf} $PassiveMode = 1
  ${OrIf} ${Silent}
    !insertmacro CloseInstalledWorkspace "inst" "grace" "Waiting for Writan to close..."
  ${EndIf}
  ; The template checks the new executable after this hook. Check the old
  ; executable too, after the update's exit grace period, so a process that
  ; could not be stopped cannot leave a live legacy binary behind.
  ${If} ${FileExists} "$INSTDIR\Workspace.exe"
    !insertmacro CheckIfAppIsRunning "$INSTDIR\Workspace.exe" "Writan"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Only retire the old registration after the new executable, uninstaller and
  ; Writan registration have been written successfully in the same directory.
  ReadRegStr $0 HKCU "${MANUKEY}\Workspace" ""
  ${If} $0 == $INSTDIR
    Delete "$INSTDIR\Workspace.exe"
    ; Tauri skips creating shortcuts under /UPDATE. Replace each legacy
    ; shortcut explicitly, while respecting a user who removed theirs.
    ${If} ${FileExists} "$SMPROGRAMS\Workspace.lnk"
      CreateShortcut "$SMPROGRAMS\Writan.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
      !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\Writan.lnk"
    ${EndIf}
    ${If} ${FileExists} "$DESKTOP\Workspace.lnk"
      CreateShortcut "$DESKTOP\Writan.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
      !insertmacro SetLnkAppUserModelId "$DESKTOP\Writan.lnk"
    ${EndIf}
    Delete "$SMPROGRAMS\Workspace.lnk"
    Delete "$DESKTOP\Workspace.lnk"
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Workspace"
    DeleteRegKey HKCU "${MANUKEY}\Workspace"
  ${EndIf}
  ; Publish the new Default Apps page immediately, including for existing users.
  WriteRegStr HKCU "Software\Writan\Capabilities" "ApplicationName" "Writan"
  WriteRegStr HKCU "Software\Writan\Capabilities" "ApplicationDescription" "Word, PowerPoint, Excel, PDF, Markdown and text files in one tabbed window"
  ; The ProgIDs are the unchanged file-association names, not the product name.
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".docx" "Word Document"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".pptx" "PowerPoint Presentation"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".xlsx" "Excel Workbook"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".csv" "CSV File"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".pdf" "PDF Document"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".md" "Markdown Document"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".markdown" "Markdown Document"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".txt" "Plain Text Document"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".tex" "LaTeX Document"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".png" "PNG Image"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".jpg" "JPEG Image"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".jpeg" "JPEG Image"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".heic" "HEIC Image"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".heif" "HEIC Image"
  WriteRegStr HKCU "Software\Writan\Capabilities\FileAssociations" ".webp" "WebP Image"
  WriteRegStr HKCU "Software\RegisteredApplications" "Writan" "Software\Writan\Capabilities"
  DeleteRegValue HKCU "Software\RegisteredApplications" "Workspace"
  DeleteRegKey HKCU "Software\Workspace\Capabilities"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro CloseInstalledWorkspace "uninst" "now" "Closing Writan..."
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\RegisteredApplications" "Writan"
  DeleteRegKey HKCU "Software\Writan\Capabilities"
!macroend
