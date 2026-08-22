@echo off
setlocal EnableExtensions
cd /d "%~dp0\..\.."
for /f "delims=" %%V in ('rustc --version') do set "RUST_VERSION=%%V"
echo %RUST_VERSION% | findstr /B /C:"rustc 1.98.0 " >nul
if errorlevel 1 (
  echo Dependency refresh requires rustc 1.98.0 to match production validation.
  exit /B 1
)
cargo update
if errorlevel 1 exit /B 1
echo.
echo Cargo.lock was intentionally refreshed. Review the dependency diff, then run:
echo   run-full-validation.cmd
