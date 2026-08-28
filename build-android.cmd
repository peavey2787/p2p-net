@echo off
setlocal
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0apps\android\build-android.ps1" -Variant Release
set "STATUS=%ERRORLEVEL%"
echo.
if "%STATUS%"=="0" (
  echo Reproducible Android release build completed successfully.
  echo Verified artifacts are under: %~dp0dist\android
) else (
  echo Reproducible Android release build failed with exit code %STATUS%.
)
echo.
echo Press any key to close this window.
pause >nul
exit /B %STATUS%
