@echo off
setlocal
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0qa\release\build-release.ps1" %*
set "STATUS=%ERRORLEVEL%"
exit /B %STATUS%
