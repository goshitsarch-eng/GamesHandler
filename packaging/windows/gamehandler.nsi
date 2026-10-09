!include "MUI2.nsh"
Name "GameHandler"
OutFile "dist\gamehandler-windows-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\GameHandler"
RequestExecutionLevel user
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Section "Install"
  SetOutPath "$INSTDIR"
  File /r "build\windows\x64\runner\Release\*"
  CreateShortcut "$SMPROGRAMS\GameHandler.lnk" "$INSTDIR\gamehandler.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\GameHandler" "DisplayName" "GameHandler"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\GameHandler" "DisplayVersion" "0.9.0-alpha.1"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\GameHandler" "UninstallString" '$"$INSTDIR\Uninstall.exe$"'
  WriteUninstaller "$INSTDIR\Uninstall.exe"
SectionEnd
Section "Uninstall"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\GameHandler"
  Delete "$SMPROGRAMS\GameHandler.lnk"
  Delete "$INSTDIR\gamehandler.exe"
  Delete "$INSTDIR\gamehandler-cli.exe"
  Delete "$INSTDIR\gamehandler_desktop.dll"
  Delete "$INSTDIR\flutter_windows.dll"
  Delete "$INSTDIR\desktop_drop_plugin.dll"
  Delete "$INSTDIR\file_selector_windows_plugin.dll"
  Delete "$INSTDIR\screen_retriever_plugin.dll"
  Delete "$INSTDIR\url_launcher_windows_plugin.dll"
  Delete "$INSTDIR\window_manager_plugin.dll"
  Delete "$INSTDIR\vcruntime140*.dll"
  Delete "$INSTDIR\msvcp140*.dll"
  Delete "$INSTDIR\concrt140.dll"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir /r "$INSTDIR\data"
  RMDir /r "$INSTDIR\share\gamehandler"
  RMDir "$INSTDIR\share"
  RMDir "$INSTDIR"
SectionEnd
