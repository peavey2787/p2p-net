@echo off
setlocal EnableExtensions EnableDelayedExpansion

set "ROOT=%~dp0"
cd /d "%ROOT%"

set "SKIP_IGNORED=0"
set "NO_INSTALL_TOOLS=0"
set "NO_CLEAN=0"
set "NO_PAUSE=0"
set "FAILED_STEP="

:parse_args
if "%~1"=="" goto args_done
if /I "%~1"=="--skip-ignored" (
  set "SKIP_IGNORED=1"
  shift
  goto parse_args
)
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
echo Unknown argument: %~1
set "FAILED_STEP=Argument parsing"
goto failed

:args_done
echo p2p-net full stable validation
echo Root: %ROOT%
echo SkipIgnored: %SKIP_IGNORED%
echo NoInstallTools: %NO_INSTALL_TOOLS%
echo NoClean: %NO_CLEAN%
echo.
echo This is the canonical Windows one-file validation runner. It auto-formats with cargo fmt and uses isolated target directories to avoid stale/incomplete build artifacts.

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
echo !RUST_VERSION! | findstr /I /C:"nightly" /C:"beta" /C:"dev" >nul
if not errorlevel 1 (
  echo p2p-net validation must run on stable Rust only. Active rustc: !RUST_VERSION!
  set "FAILED_STEP=Stable Rust check"
  goto failed
)
echo Stable Rust toolchain confirmed: !RUST_VERSION!

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

echo.
echo ==^> Refresh dependency lockfile
if exist Cargo.lock del /Q Cargo.lock
cargo generate-lockfile
if errorlevel 1 (
  set "FAILED_STEP=Refresh dependency lockfile"
  goto failed
)

cargo install --list | findstr /B /C:"cargo-audit v" >nul
if errorlevel 1 (
  if "%NO_INSTALL_TOOLS%"=="1" (
    echo cargo-audit is missing. Re-run without --no-install-tools or install it manually.
    set "FAILED_STEP=Install cargo-audit"
    goto failed
  )
  echo.
  echo ==^> Install cargo-audit
  cargo install cargo-audit --locked
  if errorlevel 1 (
    set "FAILED_STEP=Install cargo-audit"
    goto failed
  )
) else (
  echo cargo-audit already installed.
)

cargo install --list | findstr /B /C:"cargo-deny v" >nul
if errorlevel 1 (
  if "%NO_INSTALL_TOOLS%"=="1" (
    echo cargo-deny is missing. Re-run without --no-install-tools or install it manually.
    set "FAILED_STEP=Install cargo-deny"
    goto failed
  )
  echo.
  echo ==^> Install cargo-deny
  cargo install cargo-deny --locked
  if errorlevel 1 (
    set "FAILED_STEP=Install cargo-deny"
    goto failed
  )
) else (
  echo cargo-deny already installed.
)

echo.
echo ==^> Format
cargo fmt
if errorlevel 1 (
  set "FAILED_STEP=Format"
  goto failed
)

echo.
echo ==^> Dependency graph guard
if not exist Cargo.lock (
  echo Cargo.lock is missing after lockfile refresh.
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

echo.
echo ==^> Tests
set "CARGO_TARGET_DIR=%ROOT%target\full-validation\tests"
echo CARGO_TARGET_DIR=!CARGO_TARGET_DIR!
cargo test --workspace --locked -j 1
if errorlevel 1 (
  set "FAILED_STEP=Tests"
  goto failed
)

echo.
echo ==^> Dashboard feature tests
set "CARGO_TARGET_DIR=%ROOT%target\full-validation\dashboard"
echo CARGO_TARGET_DIR=!CARGO_TARGET_DIR!
cargo test --features dashboard --locked -j 1
if errorlevel 1 (
  set "FAILED_STEP=Dashboard feature tests"
  goto failed
)

echo.
echo ==^> Clippy
set "CARGO_TARGET_DIR=%ROOT%target\full-validation\clippy"
echo CARGO_TARGET_DIR=!CARGO_TARGET_DIR!
cargo clippy --workspace --all-targets --all-features --locked -j 1 -- -D warnings
if errorlevel 1 (
  set "FAILED_STEP=Clippy"
  goto failed
)

set "CARGO_TARGET_DIR="
echo.
echo ==^> Security audit
if not exist ".cargo" mkdir ".cargo"
if errorlevel 1 (
  set "FAILED_STEP=Security audit config staging"
  goto failed
)
copy /Y "qa\ci\audit.toml" ".cargo\audit.toml" >nul
if errorlevel 1 (
  set "FAILED_STEP=Security audit config staging"
  goto failed
)
cargo audit
set "AUDIT_STATUS=!ERRORLEVEL!"
if exist ".cargo\audit.toml" del /Q ".cargo\audit.toml"
rmdir ".cargo" >nul 2>&1
if not "!AUDIT_STATUS!"=="0" (
  set "FAILED_STEP=Security audit"
  goto failed
)

echo.
echo ==^> Dependency policy
cargo deny check --config qa/ci/deny.toml --help >nul 2>&1
if not errorlevel 1 (
  cargo deny check --config qa/ci/deny.toml
) else (
  cargo deny --config qa/ci/deny.toml check
)
if errorlevel 1 (
  set "FAILED_STEP=Dependency policy"
  goto failed
)

if "%SKIP_IGNORED%"=="0" (
  echo.
  echo ==^> Ignored load/soak tests
  set "CARGO_TARGET_DIR=%ROOT%target\full-validation\ignored"
  echo CARGO_TARGET_DIR=!CARGO_TARGET_DIR!
  cargo test --test multi_node_hostile --locked -j 1 -- --ignored --nocapture
  if errorlevel 1 (
    set "FAILED_STEP=Ignored load/soak tests"
    goto failed
  )
)

set "CARGO_TARGET_DIR="
echo.
echo All stable p2p-net validation checks passed.
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
