Unicode True
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
Var UpdateMode
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchDesktop
Name "Labelize Studio"
!ifndef APP_VERSION
  !error "Pass APP_VERSION from scripts/package-version.py"
!endif
!ifndef BIN_DIR
  !define BIN_DIR "..\..\target\release"
!endif
!ifndef OUTPUT_FILE
  !define OUTPUT_FILE "..\..\dist\LabelizeStudio-${APP_VERSION}-windows-x64.exe"
!endif
OutFile "${OUTPUT_FILE}"
; Keep the existing install path and uninstall key so this upgrades Printer in place.
InstallDir "$LOCALAPPDATA\Programs\Labelize Printer"
RequestExecutionLevel user
!define MUI_ICON "labelize-printer.ico"
!define MUI_UNICON "labelize-printer.ico"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "Open Labelize Studio to configure the ZDesigner queue"
!define MUI_PAGE_CUSTOMFUNCTION_SHOW UpdateFinishPage
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Function .onInit
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/LABELIZEUPDATE" $R1
  ${IfNot} ${Errors}
    StrCpy $UpdateMode "1"
  ${EndIf}
FunctionEnd

Function UpdateFinishPage
  ${If} $UpdateMode == "1"
    ShowWindow $mui.FinishPage.Run ${SW_HIDE}
  ${EndIf}
FunctionEnd

Function LaunchDesktop
  ; In-app handoff owns the pending marker. Reopen after the installer exits.
  ${If} $UpdateMode != "1"
    Exec '"$INSTDIR\labelize-printer.exe"'
  ${EndIf}
FunctionEnd

Section "Labelize Studio"
  IfFileExists "$INSTDIR\labelize-printer-daemon.exe" 0 ready
    ; Use the new client so an older executable with a missing runtime can be upgraded.
    InitPluginsDir
    SetOutPath "$PLUGINSDIR"
    File "${BIN_DIR}\labelize-printer-daemon.exe"
    ClearErrors
    ExecWait '"$PLUGINSDIR\labelize-printer-daemon.exe" stop' $0
    ${If} ${Errors}
    ${OrIf} $0 != 0
      MessageBox MB_OK "Wait for active print jobs to finish before upgrading."
      Abort
    ${EndIf}
    Sleep 500
  ready:
  SetOutPath "$INSTDIR"
  File "${BIN_DIR}\labelize-printer.exe"
  File "${BIN_DIR}\labelize-printer-updater.exe"
  File "${BIN_DIR}\labelize-printer-daemon.exe"
  File "..\..\LICENSE"
  File "..\..\THIRD_PARTY_NOTICES.md"
  File /r "..\..\licenses"
  SetOutPath "$INSTDIR\platform\windows"
  File "install-queue.ps1"
  File "autostart.ps1"
  File "run-daemon.ps1"
  SetOutPath "$INSTDIR"
  Delete "$SMPROGRAMS\Labelize Printer.lnk"
  CreateShortcut "$SMPROGRAMS\Labelize Studio.lnk" "$INSTDIR\labelize-printer.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabelizePrinter" "DisplayName" "Labelize Studio"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabelizePrinter" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabelizePrinter" "DisplayIcon" '"$INSTDIR\labelize-printer.exe",0'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabelizePrinter" "UninstallString" '"$INSTDIR\Uninstall.exe"'
SectionEnd
Section "Uninstall"
  ExecWait '"$INSTDIR\labelize-printer-daemon.exe" uninstall' $0
  ${If} $0 != 0
    MessageBox MB_OK "Remove the printer queue from Labelize Studio before uninstalling. Your saved labels and job history are retained."
    Abort
  ${EndIf}
  ExecWait '"$INSTDIR\labelize-printer-daemon.exe" stop'
  Sleep 500
  Delete "$SMPROGRAMS\Labelize Printer.lnk"
  Delete "$SMPROGRAMS\Labelize Studio.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabelizePrinter"
  Delete "$INSTDIR\labelize-printer.exe"
  Delete "$INSTDIR\labelize-printer-updater.exe"
  Delete "$INSTDIR\labelize-printer-daemon.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
  RMDir /r "$INSTDIR\platform"
  RMDir /r "$INSTDIR\licenses"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
