@echo off
setlocal
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup-android.ps1" %*
set "STATUS=%ERRORLEVEL%"
if not "%STATUS%"=="0" (
  echo.
  echo Android setup failed with exit code %STATUS%.
) else (
  echo.
  echo Android setup completed successfully.
)
echo.
pause
exit /B %STATUS%
