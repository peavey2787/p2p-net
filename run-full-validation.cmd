@echo off
setlocal EnableExtensions EnableDelayedExpansion

set "ROOT=%~dp0"
cd /d "%ROOT%"

if not defined P2P_VALIDATION_EVIDENCE_ACTIVE (
  set "P2P_VALIDATION_ORIGINAL_ARGS=%*"
  powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%ROOT%qa\evidence\run-validation-with-evidence.ps1" -Launcher "%~f0"
  set "STATUS=!ERRORLEVEL!"
  set "P2P_VALIDATION_ORIGINAL_ARGS="
  exit /B !STATUS!
)

set "NO_INSTALL_TOOLS=0"
set "NO_CLEAN=0"
set "NO_PAUSE=0"
set "FROM_STAGE=full"
set "FROM_RANK=0"
set "RESUME_NO_CLEAN=0"
set "FAILED_STEP="

:parse_args
if "%~1"=="" goto args_done
if /I "%~1"=="--no-install-tools" (
  set "NO_INSTALL_TOOLS=1"
  shift
  goto parse_args
)
if /I "%~1"=="--no-clean" (
  set "NO_CLEAN=1"
  shift
  goto parse_args
)
if /I "%~1"=="--no-pause" (
  set "NO_PAUSE=1"
  shift
  goto parse_args
)
if /I "%~1"=="--from" (
  if "%~2"=="" (
    echo --from requires a stage name.
    set "FAILED_STEP=Argument parsing"
    goto failed
  )
  set "FROM_STAGE=%~2"
  shift
  shift
  goto parse_args
)
echo Unknown argument: %~1
set "FAILED_STEP=Argument parsing"
goto failed

:args_done
set "FROM_RANK="
if /I "!FROM_STAGE!"=="full" set "FROM_RANK=0"
if /I "!FROM_STAGE!"=="lockfile" set "FROM_RANK=1"
if /I "!FROM_STAGE!"=="format" set "FROM_RANK=2"
if /I "!FROM_STAGE!"=="dependency-graph" set "FROM_RANK=3"
if /I "!FROM_STAGE!"=="tests" set "FROM_RANK=4"
if /I "!FROM_STAGE!"=="dashboard" set "FROM_RANK=5"
if /I "!FROM_STAGE!"=="clippy" set "FROM_RANK=6"
if /I "!FROM_STAGE!"=="audit" set "FROM_RANK=7"
if /I "!FROM_STAGE!"=="deny" set "FROM_RANK=8"
if not defined FROM_RANK (
  echo Unknown --from stage: !FROM_STAGE!
  echo Valid stages: lockfile, format, dependency-graph, tests, dashboard, clippy, audit, deny
  set "FAILED_STEP=Argument parsing"
  goto failed
)
if !FROM_RANK! GTR 0 if "!NO_CLEAN!"=="0" (
  set "NO_CLEAN=1"
  set "RESUME_NO_CLEAN=1"
)

echo p2p-net full stable validation
echo Root: %ROOT%
echo NoInstallTools: %NO_INSTALL_TOOLS%
echo NoClean: %NO_CLEAN%
echo ResumeFrom: !FROM_STAGE!
if "!RESUME_NO_CLEAN!"=="1" echo Resume mode preserves validation artifacts; --no-clean is implied.
echo.
echo This is the canonical Windows one-file validation runner. It verifies the committed lockfile and formatting without mutating source, and uses isolated target directories to avoid stale/incomplete build artifacts.

set "CARGO_INCREMENTAL=0"
set "CARGO_BUILD_PIPELINING=false"

where rustc >nul 2>&1
if errorlevel 1 (
  echo rustc was not found on PATH.
  set "FAILED_STEP=Rust toolchain check"
  goto failed
)
where cargo >nul 2>&1
if errorlevel 1 (
  echo cargo was not found on PATH.
  set "FAILED_STEP=Rust toolchain check"
  goto failed
)

echo.
echo ==^> Rust version
for /f "delims=" %%V in ('rustc --version') do set "RUST_VERSION=%%V"
echo !RUST_VERSION!
rustc --version >nul 2>&1
if errorlevel 1 (
  set "FAILED_STEP=Rust version"
  goto failed
)
cargo --version
if errorlevel 1 (
  set "FAILED_STEP=Cargo version"
  goto failed
)
echo !RUST_VERSION! | findstr /B /C:"rustc 1.98.0 " >nul
if errorlevel 1 (
  echo p2p-net validation requires rustc 1.98.0 exactly. Active rustc: !RUST_VERSION!
  set "FAILED_STEP=Pinned Rust check"
  goto failed
)
echo Pinned Rust toolchain confirmed: !RUST_VERSION!

for /f "tokens=2" %%H in ('rustc -vV ^| findstr /B /C:"host:"') do set "RUST_HOST=%%H"
if /I "!RUST_HOST!"=="x86_64-pc-windows-msvc" (
  echo.
  echo ==^> Windows MSVC toolchain preflight
  set "VS_INSTALL="
  set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
  if exist "!VSWHERE!" (
    for /f "usebackq delims=" %%I in (`"!VSWHERE!" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do (
      if not defined VS_INSTALL set "VS_INSTALL=%%I"
    )
  )
  if not defined VS_INSTALL (
    for %%V in (18 2022) do (
      for %%E in (BuildTools Community Professional Enterprise) do (
        if not defined VS_INSTALL if exist "%ProgramFiles(x86)%\Microsoft Visual Studio\%%V\%%E\Common7\Tools\VsDevCmd.bat" set "VS_INSTALL=%ProgramFiles(x86)%\Microsoft Visual Studio\%%V\%%E"
        if not defined VS_INSTALL if exist "%ProgramFiles%\Microsoft Visual Studio\%%V\%%E\Common7\Tools\VsDevCmd.bat" set "VS_INSTALL=%ProgramFiles%\Microsoft Visual Studio\%%V\%%E"
      )
    )
  )
  if not defined VS_INSTALL (
    echo Visual Studio C++ Build Tools were not found.
    echo Install the Desktop development with C++ workload, including the x64/x86 MSVC tools and Windows SDK.
    set "FAILED_STEP=Windows MSVC toolchain preflight"
    goto failed
  )
  if not exist "!VS_INSTALL!\Common7\Tools\VsDevCmd.bat" (
    echo VsDevCmd.bat was not found under !VS_INSTALL!.
    set "FAILED_STEP=Windows MSVC toolchain preflight"
    goto failed
  )
  call "!VS_INSTALL!\Common7\Tools\VsDevCmd.bat" -no_logo -arch=amd64 -host_arch=amd64
  if errorlevel 1 (
    echo Visual Studio developer environment initialization failed.
    set "FAILED_STEP=Windows MSVC toolchain preflight"
    goto failed
  )
  set "UCRT_LIB="
  if defined UniversalCRTSdkDir if defined UCRTVersion set "UCRT_LIB=!UniversalCRTSdkDir!Lib\!UCRTVersion!\ucrt\x64\ucrt.lib"
  if defined UCRT_LIB if not exist "!UCRT_LIB!" set "UCRT_LIB="
  if not defined UCRT_LIB if exist "%ProgramFiles(x86)%\Windows Kits\10\Lib" (
    for /f "delims=" %%U in ('where /R "%ProgramFiles(x86)%\Windows Kits\10\Lib" ucrt.lib 2^>nul ^| findstr /I /L /E /C:"\ucrt\x64\ucrt.lib"') do (
      if not defined UCRT_LIB set "UCRT_LIB=%%U"
    )
  )
  if not defined UCRT_LIB (
    echo Windows Universal CRT SDK is missing: x64 ucrt.lib was not found.
    echo In Visual Studio Installer, modify Build Tools and install Windows Universal CRT SDK plus a Windows 11 SDK.
    set "FAILED_STEP=Windows MSVC toolchain preflight"
    goto failed
  )
  echo UCRT: !UCRT_LIB!
  set "MSVC_SMOKE_DIR=%TEMP%\p2p-net-msvc-link-!RANDOM!-!RANDOM!"
  if exist "!MSVC_SMOKE_DIR!" rmdir /S /Q "!MSVC_SMOKE_DIR!"
  mkdir "!MSVC_SMOKE_DIR!"
  if errorlevel 1 (
    echo Failed to create temporary MSVC linker preflight directory.
    set "FAILED_STEP=Windows MSVC toolchain preflight"
    goto failed
  )
  >"!MSVC_SMOKE_DIR!\main.rs" echo fn main^(^) {}
  rustc --crate-name p2p_net_msvc_smoke "!MSVC_SMOKE_DIR!\main.rs" -o "!MSVC_SMOKE_DIR!\smoke.exe" >"!MSVC_SMOKE_DIR!\link.log" 2>&1
  if errorlevel 1 (
    type "!MSVC_SMOKE_DIR!\link.log"
    rmdir /S /Q "!MSVC_SMOKE_DIR!"
    echo Windows MSVC link preflight failed before validation started.
    echo Repair Build Tools so VsDevCmd provides the MSVC linker, Windows SDK, and Universal CRT libraries.
    set "FAILED_STEP=Windows MSVC toolchain preflight"
    goto failed
  )
  rmdir /S /Q "!MSVC_SMOKE_DIR!"
  echo MSVC linker and Universal CRT preflight passed.
)

if "%NO_CLEAN%"=="0" (
  echo.
  echo ==^> Clean validation artifacts
  if exist "target\full-validation" rmdir /S /Q "target\full-validation"
  cargo clean
  if errorlevel 1 (
    set "FAILED_STEP=Clean validation artifacts"
    goto failed
  )
)

if not exist Cargo.lock (
  echo Cargo.lock is missing; production validation requires the committed lockfile.
  set "FAILED_STEP=Lockfile immutability guard"
  goto failed
)
if not exist "target\full-validation" mkdir "target\full-validation"
set "VALIDATION_LOCK_BASELINE=%ROOT%target\full-validation\Cargo.lock.validation-baseline"
copy /Y Cargo.lock "!VALIDATION_LOCK_BASELINE!" >nul
if errorlevel 1 (
  echo Failed to snapshot Cargo.lock for the validation immutability guard.
  set "FAILED_STEP=Lockfile immutability guard"
  goto failed
)

set "CARGO_TARGET_DIR="
if !FROM_RANK! GEQ 2 goto ensure_tools

:stage_lockfile
echo.
echo ==^> Verify committed dependency lockfile
cargo metadata --locked --format-version 1 >nul
if errorlevel 1 (
  set "FAILED_STEP=Verify committed dependency lockfile"
  goto failed
)

:ensure_tools
for %%T in (cargo-audit:0.22.2 cargo-deny:0.20.2) do (
  for /f "tokens=1,2 delims=:" %%A in ("%%T") do (
    set "TOOL_NAME=%%A"
    set "TOOL_VERSION=%%B"
    cargo install --list | findstr /B /C:"!TOOL_NAME! v!TOOL_VERSION!:" >nul
    if errorlevel 1 (
      if "%NO_INSTALL_TOOLS%"=="1" (
        echo !TOOL_NAME! v!TOOL_VERSION! is required. Re-run without --no-install-tools or install that exact version manually.
        set "FAILED_STEP=Install !TOOL_NAME!"
        goto failed
      )
      echo.
      echo ==^> Install !TOOL_NAME! v!TOOL_VERSION!
      cargo install !TOOL_NAME! --version !TOOL_VERSION! --locked --force
      if errorlevel 1 (
        set "FAILED_STEP=Install !TOOL_NAME!"
        goto failed
      )
    ) else (
      echo !TOOL_NAME! v!TOOL_VERSION! already installed.
    )
  )
)

if !FROM_RANK! EQU 2 goto stage_format
if !FROM_RANK! EQU 3 goto stage_dependency_graph
if !FROM_RANK! EQU 4 goto stage_tests
if !FROM_RANK! EQU 5 goto stage_dashboard
if !FROM_RANK! EQU 6 goto stage_clippy
if !FROM_RANK! EQU 7 goto stage_audit
if !FROM_RANK! EQU 8 goto stage_deny

:stage_format
echo.
echo ==^> Format check
if not exist Cargo.lock (
  echo Cargo.lock is missing; format validation requires the committed lockfile.
  set "FAILED_STEP=Format"
  goto failed
)
if not exist "target\full-validation" mkdir "target\full-validation"
set "FORMAT_LOCK_BACKUP=%ROOT%target\full-validation\Cargo.lock.format-backup"
copy /Y Cargo.lock "!FORMAT_LOCK_BACKUP!" >nul
if errorlevel 1 (
  echo Failed to snapshot Cargo.lock before cargo fmt.
  set "FAILED_STEP=Format"
  goto failed
)
cargo fmt --all -- --check
set "FORMAT_STATUS=!ERRORLEVEL!"
fc /B Cargo.lock "!FORMAT_LOCK_BACKUP!" >nul
if errorlevel 1 (
  echo NOTE: cargo fmt metadata rewrote Cargo.lock; restoring the exact pre-format lockfile bytes.
  copy /Y "!FORMAT_LOCK_BACKUP!" Cargo.lock >nul
  if errorlevel 1 (
    echo Failed to restore Cargo.lock after cargo fmt.
    del /Q "!FORMAT_LOCK_BACKUP!" >nul 2>&1
    set "FAILED_STEP=Format lockfile restore"
    goto failed
  )
)
del /Q "!FORMAT_LOCK_BACKUP!" >nul 2>&1
if not "!FORMAT_STATUS!"=="0" (
  set "FAILED_STEP=Format"
  goto failed
)
cargo metadata --locked --format-version 1 >nul
if errorlevel 1 (
  echo Cargo.lock no longer matches the workspace after the read-only format check.
  set "FAILED_STEP=Post-format lockfile verification"
  goto failed
)

:stage_dependency_graph
echo.
echo ==^> Dependency graph guard
if not exist Cargo.lock (
  echo Cargo.lock is missing; production validation requires the committed lockfile.
  set "FAILED_STEP=Dependency graph guard"
  goto failed
)
for %%P in (hickory-proto hickory-resolver) do (
  findstr /L /C:"%%P" Cargo.lock >nul
  if not errorlevel 1 (
    echo %%P is present in Cargo.lock. Dependency path:
    cargo tree --target all -i %%P
    echo %%P is still present in Cargo.lock.
    set "FAILED_STEP=Dependency graph guard"
    goto failed
  )
  echo %%P is not present in Cargo.lock.
)

:stage_tests
echo.
echo ==^> Tests
echo NOTE: The Rust harness will report three long hostile/load tests as ignored in this normal phase. They are deferred, not omitted: this runner executes each one once at the end, with the soak test last.
set "CARGO_TARGET_DIR=%ROOT%target\full-validation\tests"
echo CARGO_TARGET_DIR=!CARGO_TARGET_DIR!
cargo test --workspace --locked -j 1
if errorlevel 1 (
  set "FAILED_STEP=Tests"
  goto failed
)

:stage_dashboard
echo.
echo ==^> Dashboard feature tests
set "CARGO_TARGET_DIR=%ROOT%target\full-validation\dashboard"
echo CARGO_TARGET_DIR=!CARGO_TARGET_DIR!
cargo test --features dashboard --locked -j 1
if errorlevel 1 (
  set "FAILED_STEP=Dashboard feature tests"
  goto failed
)

:stage_clippy
echo.
echo ==^> Clippy
set "CARGO_TARGET_DIR=%ROOT%target\full-validation\clippy"
echo CARGO_TARGET_DIR=!CARGO_TARGET_DIR!
cargo clippy --workspace --all-targets --all-features --locked -j 1 -- -D warnings
if errorlevel 1 (
  set "FAILED_STEP=Clippy"
  goto failed
)

:stage_audit
set "CARGO_TARGET_DIR="
echo.
echo ==^> Security audit
set "AUDIT_LOCK_BACKUP=%ROOT%target\full-validation\Cargo.lock.audit-backup"
copy /Y Cargo.lock "!AUDIT_LOCK_BACKUP!" >nul
if errorlevel 1 (
  echo Failed to snapshot Cargo.lock before cargo audit.
  set "FAILED_STEP=Security audit lockfile snapshot"
  goto failed
)
if not exist ".cargo" mkdir ".cargo"
if errorlevel 1 (
  del /Q "!AUDIT_LOCK_BACKUP!" >nul 2>&1
  set "FAILED_STEP=Security audit config staging"
  goto failed
)
copy /Y "qa\ci\audit.toml" ".cargo\audit.toml" >nul
if errorlevel 1 (
  del /Q "!AUDIT_LOCK_BACKUP!" >nul 2>&1
  set "FAILED_STEP=Security audit config staging"
  goto failed
)
cargo audit --file Cargo.lock
set "AUDIT_STATUS=!ERRORLEVEL!"
if exist ".cargo\audit.toml" del /Q ".cargo\audit.toml"
rmdir ".cargo" >nul 2>&1
fc /B Cargo.lock "!AUDIT_LOCK_BACKUP!" >nul
if errorlevel 1 (
  echo NOTE: cargo audit tooling rewrote Cargo.lock; restoring the exact pre-audit lockfile bytes.
  copy /Y "!AUDIT_LOCK_BACKUP!" Cargo.lock >nul
  if errorlevel 1 (
    del /Q "!AUDIT_LOCK_BACKUP!" >nul 2>&1
    set "FAILED_STEP=Security audit lockfile restore"
    goto failed
  )
)
del /Q "!AUDIT_LOCK_BACKUP!" >nul 2>&1
if not "!AUDIT_STATUS!"=="0" (
  set "FAILED_STEP=Security audit"
  goto failed
)
cargo metadata --locked --format-version 1 >nul
if errorlevel 1 (
  echo Cargo.lock no longer matches the workspace after the read-only security audit.
  set "FAILED_STEP=Post-audit lockfile verification"
  goto failed
)

:stage_deny
set "CARGO_TARGET_DIR="
echo.
echo ==^> Dependency policy
set "DENY_LOCK_BACKUP=%ROOT%target\full-validation\Cargo.lock.deny-backup"
copy /Y Cargo.lock "!DENY_LOCK_BACKUP!" >nul
if errorlevel 1 (
  echo Failed to snapshot Cargo.lock before cargo deny.
  set "FAILED_STEP=Dependency policy lockfile snapshot"
  goto failed
)
cargo deny check --config qa/ci/deny.toml --help >nul 2>&1
if errorlevel 1 (
  cargo deny --config qa/ci/deny.toml check
) else (
  cargo deny check --config qa/ci/deny.toml
)
set "DENY_STATUS=!ERRORLEVEL!"
fc /B Cargo.lock "!DENY_LOCK_BACKUP!" >nul
if errorlevel 1 (
  echo NOTE: cargo deny tooling rewrote Cargo.lock; restoring the exact pre-deny lockfile bytes.
  copy /Y "!DENY_LOCK_BACKUP!" Cargo.lock >nul
  if errorlevel 1 (
    del /Q "!DENY_LOCK_BACKUP!" >nul 2>&1
    set "FAILED_STEP=Dependency policy lockfile restore"
    goto failed
  )
)
del /Q "!DENY_LOCK_BACKUP!" >nul 2>&1
if not "!DENY_STATUS!"=="0" (
  set "FAILED_STEP=Dependency policy"
  goto failed
)
cargo metadata --locked --format-version 1 >nul
if errorlevel 1 (
  echo Cargo.lock no longer matches the workspace after the read-only dependency policy check.
  set "FAILED_STEP=Post-deny lockfile verification"
  goto failed
)

echo.
echo ==^> Deferred hostile relay-load test
set "CARGO_TARGET_DIR=%ROOT%target\full-validation\hostile"
echo CARGO_TARGET_DIR=!CARGO_TARGET_DIR!
cargo test --test multi_node_hostile --locked -j 1 relay_reservation_spam_does_not_panic -- --ignored --exact --nocapture
if errorlevel 1 (
  set "FAILED_STEP=Deferred hostile relay-load test"
  goto failed
)

echo.
echo ==^> Deferred hostile connection-churn test
echo CARGO_TARGET_DIR=!CARGO_TARGET_DIR!
cargo test --test multi_node_hostile --locked -j 1 circuit_open_close_spam_does_not_hang -- --ignored --exact --nocapture
if errorlevel 1 (
  set "FAILED_STEP=Deferred hostile connection-churn test"
  goto failed
)

echo.
echo ==^> Deferred one-minute soak test ^(final test^)
echo CARGO_TARGET_DIR=!CARGO_TARGET_DIR!
cargo test --test multi_node_hostile --locked -j 1 long_running_soak_node_stays_responsive -- --ignored --exact --nocapture
if errorlevel 1 (
  set "FAILED_STEP=Deferred one-minute soak test"
  goto failed
)

set "CARGO_TARGET_DIR="
fc /B Cargo.lock "!VALIDATION_LOCK_BASELINE!" >nul
if errorlevel 1 (
  echo Cargo.lock changed during validation. The validation runner must be read-only with respect to the committed dependency lockfile.
  set "FAILED_STEP=Lockfile immutability guard"
  goto failed
)
del /Q "!VALIDATION_LOCK_BASELINE!" >nul 2>&1
echo.
if !FROM_RANK! GTR 0 (
  echo p2p-net validation from !FROM_STAGE! through the final soak test passed.
  echo Earlier validation stages were intentionally skipped in resume mode.
) else (
  echo All stable p2p-net validation checks passed.
)
goto success

:failed
set "CARGO_TARGET_DIR="
echo.
echo ============================================================
echo FULL VALIDATION FAILED: %FAILED_STEP%
echo ============================================================
if "%NO_PAUSE%"=="0" if not defined CI pause
exit /B 1

:success
if "%NO_PAUSE%"=="0" if not defined CI pause
exit /B 0
