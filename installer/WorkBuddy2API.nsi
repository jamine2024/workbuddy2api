Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 32
ManifestDPIAware true

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

!ifndef APP_VERSION
  !error "APP_VERSION is required"
!endif
!ifndef PROJECT_ROOT
  !error "PROJECT_ROOT is required"
!endif
!ifndef OUTPUT_PATH
  !error "OUTPUT_PATH is required"
!endif
!ifdef TEST_INSTALL
  !define APP_NAME "WorkBuddy2API Installer Test"
  !define APP_ID "WorkBuddy2API-InstallerTest"
  !define APP_ARGS '--data-dir "$LOCALAPPDATA\WorkBuddy2API-InstallerTest"'
!else
  !define APP_NAME "WorkBuddy2API"
  !define APP_ID "WorkBuddy2API"
  !define APP_ARGS ""
!endif
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}"

Name "${APP_NAME} ${APP_VERSION}"
OutFile "${OUTPUT_PATH}"
InstallDir "$LOCALAPPDATA\Programs\${APP_ID}"
InstallDirRegKey HKCU "Software\${APP_ID}" "InstallDir"
BrandingText "WorkBuddy2API"
Icon "${PROJECT_ROOT}\rust\assets\WorkBuddy2API.ico"
UninstallIcon "${PROJECT_ROOT}\rust\assets\WorkBuddy2API.ico"
VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey /LANG=2052 "ProductName" "${APP_NAME}"
VIAddVersionKey /LANG=2052 "FileDescription" "WorkBuddy2API 当前用户安装程序"
VIAddVersionKey /LANG=2052 "FileVersion" "${APP_VERSION}.0"
VIAddVersionKey /LANG=2052 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=2052 "LegalCopyright" "WorkBuddy2API contributors"

!define MUI_ABORTWARNING
!define MUI_ICON "${PROJECT_ROOT}\rust\assets\WorkBuddy2API.ico"
!define MUI_UNICON "${PROJECT_ROOT}\rust\assets\WorkBuddy2API.ico"
!define MUI_WELCOMEPAGE_TEXT "将为当前 Windows 用户安装 WorkBuddy2API，无需管理员权限。$\r$\n$\r$\n桌面和开始菜单将创建快捷方式。账号、配置和日志保存在用户数据目录，升级和卸载时保留。"
!define MUI_FINISHPAGE_RUN "$INSTDIR\WorkBuddy2API.exe"
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchApp
!define MUI_FINISHPAGE_RUN_TEXT "启动 WorkBuddy2API"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${PROJECT_ROOT}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!define MUI_UNCONFIRMPAGE_TEXT_TOP "卸载将停止服务并移除程序及快捷方式。账号、配置、用量记录和日志默认保留在用户数据目录，下一步可选择一并删除。"
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Function LaunchApp
  Exec '"$INSTDIR\WorkBuddy2API.exe" ${APP_ARGS}'
FunctionEnd

!macro CHECK_APP_CLOSED
  ${If} ${FileExists} "$INSTDIR\WorkBuddy2API.exe"
    ClearErrors
    ExecWait '"$INSTDIR\WorkBuddy2API.exe" --shutdown ${APP_ARGS}' $0
    ${If} ${Errors}
    ${OrIf} $0 != 0
      MessageBox MB_OK|MB_ICONEXCLAMATION "无法关闭正在运行的服务，可能还有生成任务。请等待任务结束，并从托盘停止服务、退出工作台后重试。" /SD IDOK
      Abort
    ${EndIf}
  ${EndIf}
  System::Call 'kernel32::OpenMutexW(i 0x100000, i 0, w "Local\WorkBuddy2API.Desktop") p.r0'
  ${If} $0 != 0
    System::Call 'kernel32::CloseHandle(p r0)'
    MessageBox MB_OK|MB_ICONEXCLAMATION "WorkBuddy2API 已启动，可能位于托盘区。请从托盘退出已有工作台后重试。" /SD IDOK
    Abort
  ${EndIf}
!macroend

Function .onInit
  SetShellVarContext current
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "此安装包需要 64 位 Windows 10 或 Windows 11。" /SD IDOK
    Abort
  ${EndIf}
FunctionEnd

Section "WorkBuddy2API" SEC_APP
  SectionIn RO
  !insertmacro CHECK_APP_CLOSED
  SetOutPath "$INSTDIR"
  ; Explicit public payload only. Never include config, accounts, logs or runtime state.
  File "${PROJECT_ROOT}\rust\target\release\WorkBuddy2API.exe"
  File "${PROJECT_ROOT}\wb2api.exe"
  File "${PROJECT_ROOT}\rust\assets\WorkBuddy2API.ico"
  File "${PROJECT_ROOT}\LICENSE"
  File "${PROJECT_ROOT}\NOTICE.md"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortCut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\WorkBuddy2API.exe" '${APP_ARGS}' "$INSTDIR\WorkBuddy2API.ico"
  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\WorkBuddy2API.exe" '${APP_ARGS}' "$INSTDIR\WorkBuddy2API.ico"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\卸载.lnk" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\${APP_ID}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\WorkBuddy2API.exe,0"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" 25000
SectionEnd

Function un.onInit
  SetShellVarContext current
FunctionEnd

Section "Uninstall"
  !insertmacro CHECK_APP_CLOSED
  Delete "$DESKTOP\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\卸载.lnk"
  RMDir "$SMPROGRAMS\${APP_NAME}"
  !ifndef TEST_INSTALL
    Delete "$SMSTARTUP\WorkBuddy2API-Autostart.vbs"
    Delete "$SMSTARTUP\WorkBuddy2API-GUI.vbs"
  !endif
  Delete "$INSTDIR\WorkBuddy2API.exe"
  Delete "$INSTDIR\wb2api.exe"
  Delete "$INSTDIR\WorkBuddy2API.ico"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\NOTICE.md"
  Delete "$INSTDIR\Uninstall.exe"
  ; Non-recursive: user-created files remain intact.
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\${APP_ID}"
  ; 默认仍保留用户数据；只在显式确认后清目录——里面是账号凭证与 API 密钥，
  ; 静默保留等于卸载后本机仍可读回账号令牌。/SD IDNO 保证 /S 静默卸载不删数据。
  MessageBox MB_YESNO|MB_ICONQUESTION "是否同时删除用户数据目录？$\r$\n$\r$\n位置：$LOCALAPPDATA\${APP_ID}$\r$\n内含账号凭证、API 密钥、浏览器环境与运行日志。$\r$\n选择「否」将全部保留，便于以后重装继续使用。" /SD IDNO IDNO keep_userdata
    RMDir /r "$LOCALAPPDATA\${APP_ID}"
  keep_userdata:
SectionEnd
