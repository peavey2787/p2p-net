@echo off
setlocal
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0qa\release\package-crates.ps1"
set "RC=%ERRORLEVEL%"
echo.
if "%RC%"=="0" (
  echo Crates.io package validation completed successfully.
) else (
  echo Crates.io package validation failed with exit code %RC%.
)
echo.
echo Press any key to close this window.
pause >nul
exit /b %RC%
