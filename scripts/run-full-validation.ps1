param(
    [switch]$SkipIgnored,
    [switch]$NoInstallTools,
    [switch]$NoClean
)

$ErrorActionPreference = "Stop"

$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $Root

function Run-Step {
    param(
        [string]$Name,
        [scriptblock]$Command
    )

    Write-Host ""
    Write-Host "==> $Name" -ForegroundColor Cyan
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "Step failed: $Name"
    }
}


function Assert-Stable-Rust {
    $version = rustc --version
    if ($version -match "nightly|beta|dev") {
        throw "p2p-net validation must run on stable Rust only. Active rustc: $version"
    }
    Write-Host "Stable Rust toolchain confirmed: $version"
}

function Cargo-Tool-Installed {
    param([string]$Name)
    return [bool](cargo install --list | Select-String -Quiet "^$Name\s+v")
}

function Ensure-Cargo-Tool {
    param(
        [string]$Name,
        [scriptblock]$InstallCommand
    )

    if (Cargo-Tool-Installed $Name) {
        Write-Host "$Name already installed."
        return
    }

    if ($NoInstallTools) {
        throw "$Name is missing. Re-run without -NoInstallTools or install it manually."
    }

    Run-Step "Install $Name" $InstallCommand
}

function Set-Validation-Target {
    param([string]$Name)
    $env:CARGO_TARGET_DIR = Join-Path $Root "target\full-validation\$Name"
    Write-Host "CARGO_TARGET_DIR=$env:CARGO_TARGET_DIR" -ForegroundColor DarkGray
}

function Clear-Validation-Target {
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
}

function Assert-No-Rejected-Dns-Resolver {
    if (-not (Test-Path "Cargo.lock")) {
        throw "Cargo.lock is missing after lockfile refresh."
    }

    $lockText = Get-Content "Cargo.lock" -Raw
    $rejectedNames = @(
        "hickory-" + "proto",
        "hickory-" + "resolver"
    )

    foreach ($packageName in $rejectedNames) {
        $escapedName = [regex]::Escape($packageName)
        $rejectedPattern = "(?ms)^\[\[package\]\]\s+name = `"$escapedName`"\s+version = `"([^`"]+)`""
        if ($lockText -match $rejectedPattern) {
            $packageVersion = $Matches[1]
            $packageSpec = $packageName + "@" + $packageVersion
            Write-Host "$packageName $packageVersion is present in Cargo.lock. Dependency path:" -ForegroundColor Red
            # Use --target all because optional/target-gated crates can appear in
            # Cargo.lock while plain cargo tree has no active host-target path to print.
            cmd.exe /d /c "cargo tree --target all -i $packageSpec 2>&1"
            throw "$packageName $packageVersion is still present in Cargo.lock."
        }

        Write-Host "$packageName is not present in Cargo.lock."
    }
}

Write-Host "p2p-net full stable validation" -ForegroundColor Green
Write-Host "Root: $Root"
Write-Host "SkipIgnored: $SkipIgnored"
Write-Host "NoInstallTools: $NoInstallTools"
Write-Host "NoClean: $NoClean"
Write-Host ""
Write-Host "This script is the canonical one-command validation runner. It auto-formats with cargo fmt and uses isolated target directories to avoid stale/incomplete artifact errors on Windows." -ForegroundColor Yellow

# Make Windows builds less prone to stale rmeta/rlib and incremental artifact problems.
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_BUILD_PIPELINING = "false"

Run-Step "Rust version" { rustc --version; cargo --version; Assert-Stable-Rust }

if (-not $NoClean) {
    Run-Step "Clean validation artifacts" {
        Remove-Item -Recurse -Force "target\full-validation" -ErrorAction SilentlyContinue
        cargo clean
    }
}

Run-Step "Refresh dependency lockfile" {
    Remove-Item Cargo.lock -ErrorAction SilentlyContinue
    cargo generate-lockfile
}

Ensure-Cargo-Tool "cargo-audit" { cargo install cargo-audit --locked }
Ensure-Cargo-Tool "cargo-deny" { cargo install cargo-deny --locked }

Run-Step "Format" { cargo fmt }

Run-Step "Dependency graph guard" {
    Clear-Validation-Target
    Assert-No-Rejected-Dns-Resolver
}

Run-Step "Tests" {
    Set-Validation-Target "tests"
    cargo test --workspace --locked -j 1
}

Run-Step "Dashboard feature tests" {
    Set-Validation-Target "dashboard"
    cargo test --features dashboard --locked -j 1
}

Run-Step "Clippy" {
    Set-Validation-Target "clippy"
    cargo clippy --workspace --all-targets --all-features --locked -j 1 -- -D warnings
}

Run-Step "Security audit" {
    Clear-Validation-Target
    cargo audit
}

Run-Step "Dependency policy" {
    Clear-Validation-Target
    cargo deny check
}

if (-not $SkipIgnored) {
    Run-Step "Ignored load/soak tests" {
        Set-Validation-Target "ignored"
        cargo test --test multi_node_hostile --locked -j 1 -- --ignored --nocapture
    }
}

Clear-Validation-Target
Write-Host ""
Write-Host "All stable p2p-net validation checks passed." -ForegroundColor Green
