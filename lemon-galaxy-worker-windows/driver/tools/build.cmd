@echo off
rem Builds the driver and the test tool with the Enterprise WDK, then signs the driver package
rem with the LemonGalaxy test certificate (sign.ps1).
rem   build.cmd [Debug^|Release] [x64^|ARM64]
setlocal
set "CONFIG=%~1"
if "%CONFIG%"=="" set "CONFIG=Release"
set "TARGET=%~2"
if "%TARGET%"=="" set "TARGET=x64"
call "%~dp0ewdk.cmd" || exit /b 1
msbuild "%~dp0..\LemonGalaxyDriver.sln" /nologo /v:minimal /m /p:Configuration=%CONFIG% /p:Platform=%TARGET% || exit /b 1
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0sign.ps1" -Configuration %CONFIG% -Platform %TARGET% || exit /b 1
