@echo off
setlocal
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0run-android.ps1" %*
set "STATUS=%ERRORLEVEL%"
if not "%STATUS%"=="0" (
  echo.
  echo Android build/run failed with exit code %STATUS%.
) else (
  echo.
  echo Android build/run completed successfully.
)
echo.
pause
exit /B %STATUS%
