Unicode True
!include "MUI2.nsh"
!include "LogicLib.nsh"
Name "Labelize Printer"
!ifndef BIN_DIR
  !define BIN_DIR "../../target/release"
!endif
!ifndef OUTPUT_FILE
  !define OUTPUT_FILE "../../dist/LabelizePrinter-0.1.0-windows-x64.exe"
!endif
OutFile "${OUTPUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\Labelize Printer"
RequestExecutionLevel user
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\labelize-printer.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Open Labelize Printer to configure the ZDesigner queue"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Section "Labelize Printer"
  IfFileExists "$INSTDIR\labelize-printer-daemon.exe" 0 ready
    ExecWait '"$INSTDIR\labelize-printer-daemon.exe" stop' $0
    ${If} $0 != 0
      MessageBox MB_OK "Wait for active print jobs to finish before upgrading."
      Abort
    ${EndIf}
    Sleep 500
  ready:
  SetOutPath "$INSTDIR"
  File "${BIN_DIR}/labelize-printer.exe"
  File "${BIN_DIR}/labelize-printer-daemon.exe"
  File "..\..\LICENSE"
  File "..\..\THIRD_PARTY_NOTICES.md"
  File /r "..\..\licenses"
  SetOutPath "$INSTDIR\platform\windows"
  File "install-queue.ps1"
  File "autostart.ps1"
  File "run-daemon.ps1"
  SetOutPath "$INSTDIR"
  CreateShortcut "$SMPROGRAMS\Labelize Printer.lnk" "$INSTDIR\labelize-printer.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabelizePrinter" "DisplayName" "Labelize Printer"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabelizePrinter" "DisplayVersion" "0.1.0"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabelizePrinter" "UninstallString" '"$INSTDIR\Uninstall.exe"'
SectionEnd
Section "Uninstall"
  ExecWait '"$INSTDIR\labelize-printer-daemon.exe" uninstall' $0
  ${If} $0 != 0
    MessageBox MB_OK "Remove the printer queue from Labelize Printer before uninstalling. Your saved labels and job history are retained."
    Abort
  ${EndIf}
  ExecWait '"$INSTDIR\labelize-printer-daemon.exe" stop'
  Sleep 500
  Delete "$SMPROGRAMS\Labelize Printer.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabelizePrinter"
  Delete "$INSTDIR\labelize-printer.exe"
  Delete "$INSTDIR\labelize-printer-daemon.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
  RMDir /r "$INSTDIR\platform"
  RMDir /r "$INSTDIR\licenses"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
