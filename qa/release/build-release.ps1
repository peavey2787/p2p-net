Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$NoInstallTools = $false
$NoPause = $false
foreach ($arg in $args) {
    switch ($arg) {
        "--no-install-tools" { $NoInstallTools = $true }
        "--no-pause" { $NoPause = $true }
        default { throw "Unknown argument: $arg" }
    }
}

$Root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$ReproRoot = Join-Path $Root "target\reproducible-release"
$WorktreeA = Join-Path $ReproRoot "source-a"
$WorktreeB = Join-Path $ReproRoot "source-b"
$TargetA = Join-Path $ReproRoot "target-a"
$TargetB = Join-Path $ReproRoot "target-b"
$script:ExitCode = 0

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string[]]$CommandArgs
    )

    & $Command @CommandArgs
    if ($LASTEXITCODE -ne 0) {
        throw "$Command failed with exit code $LASTEXITCODE"
    }
}

function Remove-ReleaseWorktree {
    param([Parameter(Mandatory = $true)][string]$Path)

    & git worktree remove --force $Path *> $null
    if (Test-Path -LiteralPath $Path) {
        Remove-Item -LiteralPath $Path -Recurse -Force
    }
}

function Cleanup-ReleaseWorktrees {
    $oldPreference = $ErrorActionPreference
    $ErrorActionPreference = "SilentlyContinue"
    Remove-ReleaseWorktree $WorktreeA
    Remove-ReleaseWorktree $WorktreeB
    & git worktree prune *> $null
    $ErrorActionPreference = $oldPreference
}

function Assert-CleanWorktree {
    $status = @(& git status --porcelain=v1 --untracked-files=all)
    if ($LASTEXITCODE -ne 0) {
        throw "git status failed with exit code $LASTEXITCODE"
    }
    if ($status.Count -ne 0) {
        $status | ForEach-Object { Write-Host $_ -ForegroundColor Red }
        throw "official release builds require a clean Git working tree"
    }
}

function Initialize-MsvcEnvironment {
    param([Parameter(Mandatory = $true)][string]$HostTarget)

    if ($HostTarget -notlike "*-pc-windows-msvc") {
        throw "Windows release runner requires an MSVC Rust host target (found: $HostTarget)"
    }

    $vswhereCandidates = @(
        (Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"),
        (Join-Path $env:ProgramFiles "Microsoft Visual Studio\Installer\vswhere.exe")
    )
    $vswhere = $vswhereCandidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
    if (-not $vswhere) {
        throw "vswhere.exe was not found; install Visual Studio Build Tools with the MSVC and Windows SDK components"
    }

    $installLines = @(& $vswhere -latest -products "*" -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
    if ($LASTEXITCODE -ne 0 -or $installLines.Count -eq 0) {
        throw "Visual Studio Build Tools with the x64 MSVC component were not found"
    }

    $installPath = $installLines[0].Trim()
    $vsDevCmd = Join-Path $installPath "Common7\Tools\VsDevCmd.bat"
    if (-not (Test-Path -LiteralPath $vsDevCmd)) {
        throw "VsDevCmd.bat was not found at $vsDevCmd"
    }

    $commandLine = "call `"$vsDevCmd`" -no_logo -arch=amd64 -host_arch=amd64 >nul && set"
    $environmentLines = @(& $env:ComSpec /d /s /c $commandLine)
    if ($LASTEXITCODE -ne 0) {
        throw "VsDevCmd.bat failed to initialize the MSVC release environment"
    }

    foreach ($line in $environmentLines) {
        $separator = $line.IndexOf("=")
        if ($separator -gt 0) {
            $name = $line.Substring(0, $separator)
            $value = $line.Substring($separator + 1)
            [Environment]::SetEnvironmentVariable($name, $value, "Process")
        }
    }

    if (-not (Get-Command link.exe -ErrorAction SilentlyContinue)) {
        throw "MSVC link.exe is unavailable after VsDevCmd initialization"
    }
}

function Invoke-ReproducibleBuild {
    param(
        [Parameter(Mandatory = $true)][string]$SourceRoot,
        [Parameter(Mandatory = $true)][string]$TargetDir,
        [Parameter(Mandatory = $true)][string]$SourceEpoch
    )

    if (Test-Path -LiteralPath $TargetDir) {
        Remove-Item -LiteralPath $TargetDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $TargetDir -Force | Out-Null

    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
    $rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE ".rustup" }
    $encodedSeparator = [string][char]0x1f
    $rustFlags = @(
        "--remap-path-prefix=$SourceRoot=/src/p2p-net",
        "--remap-path-prefix=$TargetDir=/build/target",
        "--remap-path-prefix=$cargoHome=/cargo-home",
        "--remap-path-prefix=$rustupHome=/rustup-home",
        "-C",
        "linker=link.exe",
        "-C",
        "link-arg=/Brepro"
    )

    $previousLocation = Get-Location
    $controlledEnvNames = @(
        "RUSTFLAGS", "RUSTC", "RUSTDOC", "RUSTDOCFLAGS", "RUSTC_BOOTSTRAP",
        "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "CARGO_BUILD_JOBS", "CARGO_TERM_COLOR",
        "CARGO_NET_OFFLINE", "SOURCE_DATE_EPOCH", "TZ", "LANG", "LC_ALL",
        "CARGO_BUILD_RUSTC", "CARGO_BUILD_RUSTC_WRAPPER",
        "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTFLAGS",
        "CARGO_BUILD_TARGET", "CARGO_BUILD_TARGET_DIR",
        "CARGO_PROFILE_RELEASE_OPT_LEVEL", "CARGO_PROFILE_RELEASE_DEBUG",
        "CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", "CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS",
        "CARGO_PROFILE_RELEASE_LTO", "CARGO_PROFILE_RELEASE_PANIC",
        "CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "CARGO_PROFILE_RELEASE_RPATH",
        "CARGO_PROFILE_RELEASE_STRIP", "CARGO_PROFILE_RELEASE_INCREMENTAL",
        "CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO",
        "CC", "CXX", "AR", "CFLAGS", "CPPFLAGS", "CXXFLAGS", "LDFLAGS"
    )
    $previousEnvironment = @{}
    foreach ($name in $controlledEnvNames) {
        $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
    }

    try {
        Set-Location -LiteralPath $SourceRoot
        foreach ($name in $controlledEnvNames) {
            Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue
        }
        $env:CARGO_ENCODED_RUSTFLAGS = [string]::Join($encodedSeparator, $rustFlags)
        $env:CARGO_TARGET_DIR = $TargetDir
        $env:CARGO_INCREMENTAL = "0"
        $env:CARGO_BUILD_JOBS = "1"
        $env:CARGO_TERM_COLOR = "never"
        $env:CARGO_NET_OFFLINE = "true"
        $env:SOURCE_DATE_EPOCH = $SourceEpoch
        $env:TZ = "UTC"
        $env:LANG = "C"
        $env:LC_ALL = "C"
        Invoke-Checked -Command "cargo" -CommandArgs @(
            "build", "--release", "--locked", "--offline", "--features", "dashboard",
            "--example", "p2p_node", "-j", "1"
        )
    }
    finally {
        Set-Location $previousLocation
        foreach ($name in $controlledEnvNames) {
            [Environment]::SetEnvironmentVariable(
                $name, $previousEnvironment[$name], "Process"
            )
        }
    }
}

try {
    Set-Location -LiteralPath $Root
    if (-not (Test-Path -LiteralPath (Join-Path $Root ".git"))) {
        throw "release runner must be executed from a Git checkout"
    }

    Assert-CleanWorktree
    Cleanup-ReleaseWorktrees
    if (Test-Path -LiteralPath $ReproRoot) {
        Remove-Item -LiteralPath $ReproRoot -Recurse -Force
    }

    Write-Host "p2p-net canonical Windows release build"
    Write-Host "Root: $Root"
    Write-Host ""
    Write-Host "==> Full production validation"
    $validationArgs = @("--no-pause")
    if ($NoInstallTools) {
        $validationArgs += "--no-install-tools"
    }
    & (Join-Path $Root "run-full-validation.cmd") @validationArgs
    if ($LASTEXITCODE -ne 0) {
        throw "full production validation failed with exit code $LASTEXITCODE"
    }

    Assert-CleanWorktree

    Write-Host ""
    Write-Host "==> Resolve release identity"
    $commit = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "git rev-parse HEAD failed" }
    $tree = (& git rev-parse "HEAD^{tree}").Trim()
    if ($LASTEXITCODE -ne 0) { throw "git rev-parse tree failed" }
    $sourceEpoch = (& git show -s --format=%ct HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "git show commit timestamp failed" }
    $hostLine = @(& rustc -vV | Where-Object { $_ -like "host: *" }) | Select-Object -First 1
    if ($LASTEXITCODE -ne 0 -or -not $hostLine) { throw "could not determine rustc host target" }
    $hostTarget = $hostLine.Substring(6).Trim()
    $rustcVersion = (& rustc --version).Trim()
    if ($LASTEXITCODE -ne 0) { throw "rustc --version failed" }
    $cargoVersion = (& cargo --version).Trim()
    if ($LASTEXITCODE -ne 0) { throw "cargo --version failed" }
    if ($rustcVersion -notlike "rustc 1.98.0 *") {
        throw "release requires rustc 1.98.0 exactly (found: $rustcVersion)"
    }

    $metadataJson = & cargo metadata --locked --format-version 1 --no-deps
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata --locked failed" }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = @($metadata.packages | Where-Object { $_.name -eq "p2p-net" }) | Select-Object -First 1
    if (-not $package) { throw "could not find p2p-net package metadata" }
    $version = [string]$package.version

    Initialize-MsvcEnvironment $hostTarget
    & cargo metadata --locked --format-version 1 > $null
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata --locked failed" }

    Write-Host "Commit: $commit"
    Write-Host "Tree: $tree"
    Write-Host "Target: $hostTarget"
    Write-Host "SOURCE_DATE_EPOCH: $sourceEpoch"
    Write-Host "Rust: $rustcVersion"
    Write-Host "Cargo: $cargoVersion"

    Write-Host ""
    Write-Host "==> Create independent clean source worktrees"
    New-Item -ItemType Directory -Path $ReproRoot -Force | Out-Null
    Invoke-Checked -Command "git" -CommandArgs @(
        "worktree", "add", "--detach", $WorktreeA, $commit
    )
    Invoke-Checked -Command "git" -CommandArgs @(
        "worktree", "add", "--detach", $WorktreeB, $commit
    )

    Write-Host ""
    Write-Host "==> Reproducibility build A"
    Invoke-ReproducibleBuild $WorktreeA $TargetA $sourceEpoch

    Write-Host ""
    Write-Host "==> Reproducibility build B"
    Invoke-ReproducibleBuild $WorktreeB $TargetB $sourceEpoch

    $artifactA = Join-Path $TargetA "release\examples\p2p_node.exe"
    $artifactB = Join-Path $TargetB "release\examples\p2p_node.exe"
    if (-not (Test-Path -LiteralPath $artifactA)) { throw "release artifact A was not produced: $artifactA" }
    if (-not (Test-Path -LiteralPath $artifactB)) { throw "release artifact B was not produced: $artifactB" }

    $hashA = (Get-FileHash -LiteralPath $artifactA -Algorithm SHA256).Hash.ToLowerInvariant()
    $hashB = (Get-FileHash -LiteralPath $artifactB -Algorithm SHA256).Hash.ToLowerInvariant()

    Write-Host ""
    Write-Host "==> Verify byte-for-byte reproducibility"
    Write-Host "Build A SHA-256: $hashA"
    Write-Host "Build B SHA-256: $hashB"
    if ($hashA -ne $hashB) {
        throw "independent clean release builds are not byte-for-byte identical; targets are retained under $ReproRoot for investigation"
    }
    Write-Host "Reproducibility verified: both clean builds have identical SHA-256 digests."

    $distDir = Join-Path $Root ("dist\" + $hostTarget)
    $artifactName = "p2p_node-$version-$hostTarget.exe"
    if (Test-Path -LiteralPath $distDir) {
        Remove-Item -LiteralPath $distDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $distDir -Force | Out-Null
    Copy-Item -LiteralPath $artifactA -Destination (Join-Path $distDir $artifactName)

    @("$hashA  $artifactName") | Set-Content -LiteralPath (Join-Path $distDir "SHA256SUMS.txt") -Encoding ASCII
    @(
        "project=p2p-net",
        "version=$version",
        "git_commit=$commit",
        "git_tree=$tree",
        "source_date_epoch=$sourceEpoch",
        "rustc=$rustcVersion",
        "cargo=$cargoVersion",
        "target=$hostTarget",
        "linker=link.exe",
        "profile=release",
        "features=default,dashboard",
        "artifact=$artifactName",
        "sha256=$hashA",
        "reproducibility=verified-two-independent-clean-worktrees",
        "validation=run-full-validation.cmd-passed"
    ) | Set-Content -LiteralPath (Join-Path $distDir "BUILD-MANIFEST.txt") -Encoding ASCII

    Write-Host ""
    Write-Host "==> Release output"
    Write-Host "Artifact: $(Join-Path $distDir $artifactName)"
    Write-Host "Manifest: $(Join-Path $distDir 'BUILD-MANIFEST.txt')"
    Write-Host "Checksums: $(Join-Path $distDir 'SHA256SUMS.txt')"
    Write-Host ""
    Write-Host "Canonical Windows release build completed successfully."
}
catch {
    Write-Host ""
    Write-Host "============================================================" -ForegroundColor Red
    Write-Host "RELEASE BUILD FAILED" -ForegroundColor Red
    Write-Host "============================================================" -ForegroundColor Red
    Write-Host $_.Exception.Message -ForegroundColor Red
    $script:ExitCode = 1
}
finally {
    Set-Location -LiteralPath $Root
    Cleanup-ReleaseWorktrees
}

if (-not $NoPause -and -not $env:CI) {
    Write-Host ""
    if ($script:ExitCode -eq 0) {
        Read-Host "Release build complete. Press Enter to close" | Out-Null
    }
    else {
        Read-Host "Release build failed. Press Enter to close" | Out-Null
    }
}

exit $script:ExitCode
