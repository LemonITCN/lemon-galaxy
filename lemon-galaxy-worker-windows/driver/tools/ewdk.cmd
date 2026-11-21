@echo off
rem Sets up the Enterprise WDK build environment, mounting its ISO when needed.
rem LG_EWDK_ISO points at the image (default D:\dev\EWDK_28000.iso).
if defined EnterpriseWDK exit /b 0
if not defined LG_EWDK_ISO set "LG_EWDK_ISO=D:\dev\EWDK_28000.iso"
set "LG_EWDK_DRIVE="
for /f "usebackq delims=" %%d in (`powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0ewdk-mount.ps1" -Image "%LG_EWDK_ISO%"`) do set "LG_EWDK_DRIVE=%%d:"
if not exist "%LG_EWDK_DRIVE%\BuildEnv\SetupBuildEnv.cmd" (
  echo EWDK not found: %LG_EWDK_ISO%
  exit /b 1
)
call "%LG_EWDK_DRIVE%\BuildEnv\SetupBuildEnv.cmd" amd64 >nul
exit /b 0
